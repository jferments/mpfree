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
//! - **Went to:** each line says what took its message: the armed track
//!   (and the pad its note plays with a kit), MIDI Learn and what it
//!   moved, the clock follower, nothing, or "ignored"; a line going out
//!   says which track sent it or that it is MPFree's clock.
//!
//! **Debug mode** ([`DevMonitor`], the Dev tab's MIDI page) is the same
//! section with two more sources: the MPC's own controller (its pads,
//! buttons, Q-Links and wheel as the app read them, and the lights, pad
//! colours, keep-alive and CV values it wrote) and MPFree's own events.
//! Its chips pick sources rather than kinds of message.

use std::cell::Cell;
use std::time::Duration;

use mpfree_controller::mpc_one::{button_name, ControlEvent};
use mpfree_midi::monitor::{
    Dir, Event, Filter, Kind, Monitor, Mpc, Seen, Steady, To, What, MPC, SYSTEM,
};
use mpfree_midi::{Msg, PortId};
use mpfree_mixer_ui::{IoInput, IoReply, IoSection};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Theme, Touch, Widget};

use crate::form::{BANNER_H, GAP};
use crate::now::{self, NowData};
use crate::{note_name, IoModel, Shared};

/// A chip: its label, its width, and the kinds it shows or hides.
type Chip = (&'static str, i32, u16);

fn chips_for(debug: bool) -> &'static [Chip] {
    if debug {
        &DEBUG_CHIPS
    } else {
        &KIND_CHIPS
    }
}

/// The I/O view's chips: one per kind of message a port carries.
const KIND_CHIPS: [Chip; 6] = [
    ("Notes", 78, Kind::Notes.bit()),
    ("CC", 56, Kind::Controls.bit()),
    ("Program", 96, Kind::Programs.bit()),
    ("Bend & pressure", 158, Kind::Bends.bit()),
    ("Clock", 74, Kind::Clock.bit()),
    ("SysEx", 78, Kind::System.bit()),
];

/// Debug mode's chips: one per source (a port's messages but its clock,
/// and each of the debug sources), then the clock and the keep-alive.
const DEBUG_CHIPS: [Chip; 7] = [
    (
        "Ports",
        74,
        Kind::Notes.bit()
            | Kind::Controls.bit()
            | Kind::Programs.bit()
            | Kind::Bends.bit()
            | Kind::System.bit(),
    ),
    ("MPC controls", 128, Kind::MpcControls.bit()),
    ("Lights & colours", 156, Kind::Lights.bit()),
    ("CV out", 84, Kind::Cv.bit()),
    ("System", 84, Kind::Events.bit()),
    ("Clock", 74, Kind::Clock.bit()),
    ("Keep-alive", 110, Kind::KeepAlive.bit()),
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

/// The list's fixed columns: time, port and way, channel, bytes. The
/// message and where it went share the rest.
#[derive(Clone, Copy)]
struct Cols {
    time: i32,
    port: i32,
    ch: i32,
    bytes: i32,
}

/// Narrower below 1000 px (the Dev page's half of the screen).
fn cols(w: i32) -> Cols {
    if w >= 1000 {
        Cols {
            time: 104,
            port: 200,
            ch: 70,
            bytes: 120,
        }
    } else {
        Cols {
            time: 96,
            port: 124,
            ch: 52,
            bytes: 120,
        }
    }
}

/// The Dev page's hint line (debug mode).
pub(crate) const DEBUG_HINT: &str =
    "Tap a chip: show or hide a source  ·  Hold stops the list  ·  the I/O view's Monitor has the wheel";

/// The hint line while the Monitor shows.
pub(crate) const HINT: &str = "Tap a light: only that port and channel  ·  wheel: scroll back  ·  Hold stops the list  ·  Save log: to the stick";

/// The hint line while the Now view shows.
pub(crate) const NOW_HINT: &str = "Tap a port: what it holds now  ·  wheel or ‹ ›: channel  ·  Hold freezes the view  ·  the I/O icon or TRACK MIX closes";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    /// A chip, by its place in the chips shown.
    Chip(usize),
    Dir(Dir),
    Hold,
    Clear,
    Port(PortId, Dir),
    Light(PortId, Dir, u8),
    List,
    /// List (false) or Now (true).
    View(bool),
    /// Save log.
    Save,
    /// The Now view's port and way, by its place in [`now::streams`].
    Stream(usize),
    /// The Now view's channel, one down (-1) or up (1).
    Channel(i32),
    /// Send note-offs to MIDI out.
    NoteOffs,
}

struct Layout {
    banner: Option<Rect>,
    chips: Vec<(Hit, Rect)>,
    grid_head: Rect,
    /// Each port shown and its row (the in half on top).
    ports: Vec<(PortId, Rect)>,
    list_head: Rect,
    list: Rect,
    /// The Now view's cards and its channel's label, while it shows.
    now: Option<(now::Cards, Rect)>,
}

/// List | Now: each half's width.
const SEG_W: i32 = 72;
/// The Now view's channel arrows and label.
const ARROW_W: i32 = 44;
const CH_W: i32 = 70;

/// The Save log button's width.
const SAVE_W: i32 = 100;

/// A Now view port chip's width for `text` (laid out before it is drawn).
fn chip_w(text: &str) -> i32 {
    28 + 9 * text.chars().count() as i32
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
pub(crate) fn clock_text(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0) as u64;
    format!("{m}:{:06.3}", t - m as f64 * 60.0)
}

pub(crate) fn way(d: Dir) -> &'static str {
    match d {
        Dir::In => "in",
        Dir::Out => "out",
    }
}

/// A port's name and way, as the list says it ("Keystep 37 in").
pub(crate) fn port_text(m: &Monitor, port: PortId, d: Dir) -> String {
    if port == MPC {
        return format!("MPC {}", way(d));
    }
    if port == SYSTEM {
        return "System".to_string();
    }
    match m.port_name(port) {
        Some(n) => format!("{n} {}", way(d)),
        None => format!("Port {} {}", port.0 + 1, way(d)),
    }
}

/// The controllers most gear agrees on.
pub(crate) fn cc_name(cc: u8) -> Option<&'static str> {
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
        What::Mpc(m) => mpc_words(m),
        What::Event(e) => event_words(e, |p| format!("port {}", p.0 + 1)),
    }
}

/// A button as its panel prints it.
fn button_text(note: u8) -> String {
    button_name(note).map_or_else(|| format!("Button {note}"), str::to_string)
}

/// The MPC's own controller, in words (debug mode).
fn mpc_words(m: Mpc) -> String {
    match m {
        Mpc::Control { ev, .. } => match ev {
            ControlEvent::PadDown { pad, velocity } => {
                format!("Pad {} down · vel {velocity}", pad + 1)
            }
            ControlEvent::PadUp { pad } => format!("Pad {} up", pad + 1),
            ControlEvent::PadPressure { pad, pressure } => {
                format!("Pad {} pressure · {pressure}", pad + 1)
            }
            ControlEvent::QLinkTurn { knob, delta } => {
                format!("Q-Link {} turned {delta:+}", knob + 1)
            }
            ControlEvent::QLinkTouch { knob, touched } => format!(
                "Q-Link {} {}",
                knob + 1,
                if touched { "touched" } else { "let go" }
            ),
            ControlEvent::DataWheel { delta } => format!("Data wheel {delta:+}"),
            ControlEvent::Button { note, pressed } => format!(
                "{} {}",
                button_text(note),
                if pressed { "pressed" } else { "released" }
            ),
            ControlEvent::KeepAliveReply { token } => format!(
                "Keep-alive reply · token {:02X} {:02X} {:02X} {:02X}",
                token[0], token[1], token[2], token[3]
            ),
        },
        Mpc::Light { button, value } => {
            let level = match value {
                0 => "off".to_string(),
                127 => "on".to_string(),
                v => format!("at {v}"),
            };
            format!("Light {} {level}", button_text(button))
        }
        Mpc::PadColour { pad, rgb } => {
            format!("Pad {} colour · {} {} {}", pad + 1, rgb[0], rgb[1], rgb[2])
        }
        Mpc::KeepAlive => "Keep-alive ping".to_string(),
        Mpc::Cv { out, value } => {
            let volts = f32::from(value) / f32::from(mpfree_midi::cv_wire::FULL_SCALE)
                * mpfree_midi::setup::MAX_VOLTS;
            format!(
                "CV {} = {value} · {volts:.3} V if full scale is {} V",
                out + 1,
                mpfree_midi::setup::MAX_VOLTS
            )
        }
    }
}

/// One of MPFree's events, in words (debug mode), naming ports by `name`.
fn event_words(e: Event, name: impl Fn(PortId) -> String) -> String {
    match e {
        Event::Plugged(p) => format!("Plugged in: {}", name(p)),
        Event::Unplugged { port, ended } => match ended {
            0 => format!("Unplugged: {}", name(port)),
            1 => format!("Unplugged: {} · 1 held note let go", name(port)),
            n => format!("Unplugged: {} · {n} held notes let go", name(port)),
        },
        Event::OutStopped { port } => format!(
            "MIDI out to {} stopped: its tracks play on the pads again",
            name(port)
        ),
        Event::Dropped { port, n } => {
            format!("{n} messages to {} dropped: its queue was full", name(port))
        }
    }
}

/// A line's message in words, naming the ports an event is about.
pub(crate) fn line_words(mon: &Monitor, s: &Seen) -> String {
    match s.what {
        What::Event(e) => event_words(e, |p| match mon.port_name(p) {
            Some(n) if p == mpfree_midi::DIN => format!("{n} (5-pin jacks)"),
            Some(n) => n.to_string(),
            None => format!("port {}", p.0 + 1),
        }),
        _ => words(s),
    }
}

/// How a line's "went to" is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Went {
    /// Played or moved something.
    Took,
    /// MIDI Learn.
    Learn,
    /// The clock follower, or MPFree's own clock.
    Clock,
    /// Nothing took it.
    Nothing,
    /// Going out: who sent it.
    Sent,
}

/// Where a line's message went, in words ([`To`]); empty while unsaid.
pub(crate) fn went_to(m: &IoModel, to: To) -> (String, Went) {
    let track = |t: u8| {
        m.live
            .tracks
            .get(usize::from(t))
            .cloned()
            .unwrap_or_else(|| format!("Track {}", u16::from(t) + 1))
    };
    let param = |a| {
        m.setup
            .learn
            .iter()
            .position(|l| l.target == a)
            .and_then(|i| m.live.learn_names.get(i))
            .cloned()
            .unwrap_or_else(|| a.to_string())
    };
    match to {
        To::Unsaid => (String::new(), Went::Nothing),
        To::Track {
            track: t,
            pad: None,
        } => (format!("{}, the armed track", track(t)), Went::Took),
        To::Track {
            track: t,
            pad: Some(p),
        } => (format!("{}, pad {}", track(t), p + 1), Went::Took),
        To::NoPad { track: t } => (format!("nothing: no pad on {}", track(t)), Went::Nothing),
        To::Routed { track: t } => (format!("{}, by its route", track(t)), Went::Took),
        // Thru sends only while MIDI out is on.
        To::Thru { to: p } if m.live.built.notes_out => (
            format!("thru to {}", port_text(&m.monitor, p, Dir::Out)),
            Went::Took,
        ),
        To::Thru { .. } => ("thru: not sent, MIDI out is off".to_string(), Went::Nothing),
        To::Learned(a) => (format!("Learn: {}", param(a)), Went::Learn),
        To::Took(a) => (format!("Learn took it: {}", param(a)), Went::Learn),
        To::LearnOnly => ("not played: Learn only".to_string(), Went::Nothing),
        To::Follows => ("clock: the tempo follows it".to_string(), Went::Clock),
        To::NotFollowed => ("not followed".to_string(), Went::Nothing),
        To::Nothing => ("nothing takes it".to_string(), Went::Nothing),
        To::Ignored => ("ignored".to_string(), Went::Nothing),
        To::PortIgnores => ("ignored: this port's setting".to_string(), Went::Nothing),
        To::SentBy(t) => (format!("sent by {}", track(t)), Went::Sent),
        To::Clock => ("MPFree's clock".to_string(), Went::Clock),
        To::PortPage => ("sent from its page".to_string(), Went::Sent),
        To::ThruFrom(p) => (
            format!("thru from {}", port_text(&m.monitor, p, Dir::In)),
            Went::Sent,
        ),
    }
}

pub(crate) fn went_ink(t: &Theme, w: Went) -> Color {
    match w {
        Went::Took => t.text,
        Went::Learn => t.accent2,
        Went::Clock => t.good,
        Went::Nothing => t.warn,
        Went::Sent => t.text_dim,
    }
}

/// The bytes a line shows: a channel message's own, a SysEx's start.
pub(crate) fn bytes_text(w: &What) -> String {
    let (b, n) = w.bytes();
    let mut s = b[..n]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    if matches!(
        w,
        What::SysEx { .. } | What::Mpc(Mpc::PadColour { .. } | Mpc::KeepAlive)
    ) {
        s.push_str(" … F7");
    }
    s
}

pub(crate) struct MonitorSection {
    model: Shared,
    /// Debug mode: the MPC's own controller and MPFree's events too, and
    /// chips by source ([`DEBUG_CHIPS`]).
    debug: bool,
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
    /// The Now view shows (not in debug mode).
    now_view: bool,
    /// The Now view's port and way, and channel, as picked; `None`: the
    /// newest.
    pick: Option<(PortId, Dir)>,
    pick_ch: Option<u8>,
    /// The Now view as Hold froze it.
    frozen: Option<Box<NowData>>,
    /// What the last tap did, for the hint line (Save log's file).
    note: Option<String>,
    /// Save log asked; the app's answer is still to come.
    saving: bool,
}

impl MonitorSection {
    pub fn new(model: Shared) -> MonitorSection {
        MonitorSection {
            model,
            debug: false,
            filter: Filter::ALL,
            anchor: None,
            held: false,
            now: Duration::ZERO,
            live: u64::MAX,
            down: None,
            fit: Cell::new(10),
            now_view: false,
            pick: None,
            pick_ch: None,
            frozen: None,
            note: None,
            saving: false,
        }
    }

    /// Whether the Now view shows.
    fn now_shows(&self) -> bool {
        self.now_view && !self.debug
    }

    /// The Now view's port and way, and channel.
    fn now_pick(&self, m: &IoModel) -> ((PortId, Dir), u8) {
        let s = self
            .pick
            .filter(|s| now::streams(&m.monitor).contains(s))
            .unwrap_or_else(|| now::auto_stream(&m.monitor));
        let c = self
            .pick_ch
            .unwrap_or_else(|| now::auto_channel(&m.monitor, s));
        (s, c)
    }

    /// What the Now view shows: as Hold froze it, or now.
    fn now_data(&self, m: &IoModel) -> NowData {
        if let Some(f) = &self.frozen {
            return (**f).clone();
        }
        let (s, c) = self.now_pick(m);
        now::data(m, s, c, self.clock(m))
    }

    /// Save log: asks the app to keep the list as shown in a new file in
    /// the data folder's `files/midi-logs/`. The app's answer comes to the
    /// hint line ([`MonitorSection::tick`]).
    fn save(&mut self) {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        if !m.live.saves_logs {
            self.note = Some("Save log: no data folder to keep it in this run".into());
            return;
        }
        if self.saving {
            self.note = Some("Save log: still saving the last one".into());
            return;
        }
        let lines: Vec<(u64, Seen)> = m
            .monitor
            .newest()
            .filter(|(_, s)| self.shows(s))
            .map(|(n, s)| (n, *s))
            .collect();
        if lines.is_empty() {
            self.note = Some("Save log: nothing listed to save".into());
            return;
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let (stamp, when) = crate::log::stamp(secs);
        let text = crate::log::saved(&m, &lines, &when, &self.filter_text(&m.monitor));
        m.live.log_saved = None;
        m.asks.save_log = Some(crate::LogToSave {
            stamp,
            text,
            lines: lines.len(),
        });
        self.saving = true;
    }

    /// Whether a line is listed: the filter's, and the debug sources only
    /// in debug mode (the app lists them while the developer tools are on,
    /// for the Dev page; the I/O view keeps to the ports).
    fn shows(&self, s: &Seen) -> bool {
        self.filter.matches(s) && (self.debug || !matches!(s.what, What::Mpc(_) | What::Event(_)))
    }

    /// The chips shown: by kind, or by source in debug mode.
    fn chip_set(&self) -> &'static [Chip] {
        chips_for(self.debug)
    }

    fn chip_on(&self, i: usize) -> bool {
        let mask = self.chip_set()[i].2;
        self.filter.kinds & mask == mask
    }

    /// The time the lights read: the app's clock once it has said when
    /// that started, else the screen's.
    fn clock(&self, m: &IoModel) -> f64 {
        m.monitor.now(self.now.as_secs_f64())
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
        let right = r.x + r.w;
        // List | Now (the I/O view's Monitor only).
        if !self.debug {
            chips.push((Hit::View(false), Rect::new(x, y, SEG_W, CHIP_H)));
            chips.push((Hit::View(true), Rect::new(x + SEG_W, y, SEG_W, CHIP_H)));
            x += 2 * SEG_W + 14;
        }
        if self.now_shows() {
            // The ports and ways, then the channel and Hold on the right.
            let hold = Rect::new(right - BUTTON_W, y, BUTTON_W, CHIP_H);
            let up = Rect::new(hold.x - 14 - ARROW_W, y, ARROW_W, CHIP_H);
            let label = Rect::new(up.x - CH_W, y, CH_W, CHIP_H);
            let down = Rect::new(label.x - ARROW_W, y, ARROW_W, CHIP_H);
            for (i, &(p, d)) in now::streams(&m.monitor).iter().enumerate() {
                let w = chip_w(&port_text(&m.monitor, p, d));
                if x + w > down.x - 14 {
                    break;
                }
                chips.push((Hit::Stream(i), Rect::new(x, y, w, CHIP_H)));
                x += w + CHIP_GAP;
            }
            chips.push((Hit::Channel(-1), down));
            chips.push((Hit::Channel(1), up));
            chips.push((Hit::Hold, hold));
            y += CHIP_H + 12;
            let cards = now::cards(Rect::new(r.x, y, r.w, (r.y + r.h - y).max(0)));
            chips.push((Hit::NoteOffs, cards.note_offs));
            let none = Rect::new(r.x, y, 0, 0);
            return Layout {
                banner,
                chips,
                grid_head: none,
                ports: Vec::new(),
                list_head: none,
                list: none,
                now: Some((cards, label)),
            };
        }
        for (i, &(_, w, _)) in self.chip_set().iter().enumerate() {
            chips.push((Hit::Chip(i), Rect::new(x, y, w, CHIP_H)));
            x += w + CHIP_GAP;
        }
        // The ways and the buttons (Hold, Clear and, on the I/O view, Save
        // log) follow on the same line when they fit, otherwise on a line
        // of their own.
        let dirs_w: i32 = DIR_CHIPS.iter().map(|c| c.1 + CHIP_GAP).sum();
        let save_w = if self.debug { 0 } else { SAVE_W + CHIP_GAP };
        let buttons_w = 2 * BUTTON_W + CHIP_GAP + save_w;
        if x + 14 + dirs_w + buttons_w > right {
            y += CHIP_H + 8;
            x = r.x;
        } else {
            x += 14;
        }
        for (_, w, d) in DIR_CHIPS {
            chips.push((Hit::Dir(d), Rect::new(x, y, w, CHIP_H)));
            x += w + CHIP_GAP;
        }
        let mut bx = right;
        if !self.debug {
            bx -= SAVE_W;
            chips.push((Hit::Save, Rect::new(bx, y, SAVE_W, CHIP_H)));
            bx -= CHIP_GAP;
        }
        chips.push((Hit::Clear, Rect::new(bx - BUTTON_W, y, BUTTON_W, CHIP_H)));
        chips.push((
            Hit::Hold,
            Rect::new(bx - 2 * BUTTON_W - CHIP_GAP, y, BUTTON_W, CHIP_H),
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
            now: None,
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
        for (n, s) in m.newest().filter(|(_, s)| self.shows(s)) {
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
            .filter(|(_, s)| self.shows(s))
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
        match h {
            Hit::View(now) => {
                if self.now_view == now {
                    return IoReply::Handled;
                }
                self.now_view = now;
                self.frozen = None;
                return IoReply::Redraw;
            }
            Hit::Save => {
                self.save();
                return IoReply::Redraw;
            }
            Hit::Stream(i) => {
                let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                let s = now::streams(&m.monitor).get(i).copied();
                drop(m);
                if s.is_none() || self.pick == s {
                    return IoReply::Handled;
                }
                self.pick = s;
                self.pick_ch = None;
                self.frozen = None;
                return IoReply::Redraw;
            }
            Hit::Channel(d) => return self.step_channel(d),
            Hit::NoteOffs => {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                let d = {
                    let (s, c) = self.now_pick(&m);
                    now::data(&m, s, c, self.clock(&m))
                };
                if !now::can_end(&d) {
                    self.note = Some(if d.out_on {
                        "No notes are still on going out of MIDI out".into()
                    } else {
                        "MIDI out is off in this run: no notes to end".into()
                    });
                    return IoReply::Handled;
                }
                m.asks.note_offs = true;
                self.frozen = None;
                self.note = Some("Note-offs sent for the notes MPFree left on".into());
                return IoReply::Redraw;
            }
            Hit::Hold if self.now_shows() => {
                if self.frozen.take().is_none() {
                    let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                    self.frozen = Some(Box::new(self.now_data(&m)));
                }
                return IoReply::Redraw;
            }
            _ => {}
        }
        let f = &mut self.filter;
        match h {
            Hit::Chip(i) => {
                let mask = chips_for(self.debug)[i].2;
                if f.kinds & mask == mask {
                    f.kinds &= !mask;
                } else {
                    f.kinds |= mask;
                }
            }
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
            Hit::View(_) | Hit::Save | Hit::Stream(_) | Hit::Channel(_) | Hit::NoteOffs => {
                return IoReply::Handled
            }
        }
        IoReply::Redraw
    }

    /// The Now view's channel, `d` up or down (round from 16 to 1).
    fn step_channel(&mut self, d: i32) -> IoReply {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let (_, c) = self.now_pick(&m);
        drop(m);
        self.pick_ch = Some((i32::from(c) + d).rem_euclid(16) as u8);
        self.frozen = None;
        IoReply::Redraw
    }

    /// A number that changes when anything the screen shows changes.
    fn live_now(&self, m: &IoModel, now: f64) -> u64 {
        if self.now_shows() {
            if self.frozen.is_some() {
                return 1;
            }
            let (s, c) = self.now_pick(m);
            return now::live(&now::data(m, s, c, now)) ^ m.monitor.total();
        }
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
        let hidden: Vec<&str> = self
            .chip_set()
            .iter()
            .enumerate()
            .filter(|&(i, _)| !self.chip_on(i))
            .map(|(_, c)| c.0)
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
            let why = if mon.is_empty() && self.debug {
                "Nothing yet: press a pad, or play a keyboard on MIDI In or USB"
            } else if mon.is_empty() {
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
        let c = cols(l.list.w);
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
                c.time,
                row,
                FontId::Mono,
                t.text_faint,
                &clock_text(s.t),
            );
            let ink = match (s.port, s.dir) {
                (SYSTEM, _) => t.text_dim,
                (_, Dir::In) => t.text,
                (_, Dir::Out) => t.accent,
            };
            let port = port_text(mon, s.port, s.dir);
            let x = col(cx, x, c.port, row, FontId::Sans, ink, &port);
            let ch = s
                .what
                .channel()
                .map_or(String::new(), |c| format!("Ch {}", c + 1));
            let x = col(cx, x, c.ch, row, FontId::Sans, t.text_dim, &ch);
            // The message and where it went share what is left; a line
            // that went nowhere in particular (the MPC's own, an event)
            // has it all.
            let rest = row.x + row.w - c.bytes - x;
            let (went, w) = went_to(m, s.to);
            let words_w = if went.is_empty() {
                rest
            } else {
                rest * 11 / 20
            };
            let kind_ink = match s.what {
                What::Arriving(_) | What::Stopped(..) => t.text_dim,
                _ => t.text,
            };
            let x = col(
                cx,
                x,
                words_w,
                row,
                FontId::Sans,
                kind_ink,
                &line_words(mon, s),
            );
            col(
                cx,
                x,
                rest - words_w,
                row,
                FontId::Sans,
                went_ink(&t, w),
                &went,
            );
            cx.text(
                FontId::Mono,
                t.size_small,
                Rect::new(row.x + row.w - c.bytes, row.y, c.bytes - 8, row.h),
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
        Some(if self.debug {
            DEBUG_HINT
        } else if self.now_shows() {
            NOW_HINT
        } else {
            HINT
        })
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let l = self.layout(&m, r);
        let mut out = Vec::new();
        for &(h, cr) in &l.chips {
            let t = match h {
                Hit::Chip(i) => {
                    let id = if self.debug {
                        "monitor-source"
                    } else {
                        "monitor-kind"
                    };
                    Target::tap(id, i, self.chip_set()[i].0, cr)
                }
                Hit::Dir(d) => {
                    let i = DIR_CHIPS.iter().position(|c| c.2 == d).unwrap_or(0);
                    Target::tap("monitor-way", i, DIR_CHIPS[i].0, cr)
                }
                Hit::Hold => Target::tap("monitor-hold", 0, "Hold", cr),
                Hit::Clear => Target::tap("monitor-clear", 0, "Clear", cr),
                // The other view (the one showing is no control).
                Hit::View(now) if now != self.now_view => Target::tap(
                    "monitor-view",
                    usize::from(now),
                    if now { "Now" } else { "List" },
                    cr,
                ),
                Hit::Save => {
                    Target::tap("monitor-save", 0, "Save log", cr).disabled_if(!m.live.saves_logs)
                }
                Hit::Stream(i) => {
                    let label = now::streams(&m.monitor)
                        .get(i)
                        .map_or(String::new(), |&(p, d)| port_text(&m.monitor, p, d));
                    Target::tap("monitor-stream", i, label, cr)
                }
                Hit::Channel(d) => Target::tap(
                    "monitor-channel",
                    usize::from(d > 0),
                    if d > 0 {
                        "Next channel"
                    } else {
                        "Channel before"
                    },
                    cr,
                ),
                Hit::NoteOffs => {
                    let d = self.now_data(&m);
                    Target::tap("monitor-note-offs", 0, "Send note-offs to MIDI out", cr)
                        .disabled_if(!now::can_end(&d))
                }
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
                Hit::Chip(i) => {
                    Self::chip(cx, cr, self.chip_set()[i].0, self.chip_on(i));
                }
                Hit::Dir(d) => {
                    let name = DIR_CHIPS.iter().find(|c| c.2 == d).map_or("", |c| c.0);
                    Self::chip(cx, cr, name, self.filter.shows_dir(d));
                }
                Hit::View(now) => {
                    if !now {
                        let seg = Rect::new(cr.x, cr.y, 2 * SEG_W, CHIP_H);
                        cx.canvas.fill_round_rect(seg, 10, t.surface);
                    }
                    let on = now == self.now_view;
                    if on {
                        cx.canvas.fill_round_rect(cr.inset(3), 8, t.accent);
                    }
                    cx.text(
                        FontId::Bold,
                        t.size_small,
                        cr,
                        Align::Center,
                        if on { t.on_accent } else { t.text_dim },
                        if now { "Now" } else { "List" },
                    );
                }
                Hit::Save => {
                    let on = m.live.saves_logs;
                    cx.canvas
                        .fill_round_rect(cr, 10, if on { t.raised } else { t.surface });
                    cx.text(
                        FontId::Bold,
                        t.size_small,
                        cr,
                        Align::Center,
                        if on { t.text } else { t.text_faint },
                        "Save log",
                    );
                }
                Hit::Stream(i) => {
                    if let Some(&(p, d)) = now::streams(&m.monitor).get(i) {
                        let picked = self.now_pick(&m).0 == (p, d);
                        Self::chip(cx, cr, &port_text(&m.monitor, p, d), picked);
                    }
                }
                Hit::Channel(d) => {
                    cx.canvas.fill_round_rect(cr, 10, t.raised);
                    cx.text(
                        FontId::Bold,
                        t.size_body,
                        cr,
                        Align::Center,
                        t.text,
                        if d > 0 { "›" } else { "‹" },
                    );
                }
                Hit::Hold if self.now_shows() => {
                    let on = self.frozen.is_some();
                    let (fill, ink) = if on {
                        (t.warn, t.bg)
                    } else {
                        (t.raised, t.text)
                    };
                    cx.canvas.fill_round_rect(cr, 10, fill);
                    cx.text(FontId::Bold, t.size_small, cr, Align::Center, ink, "Hold");
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
        if let Some((cards, label)) = l.now {
            let d = self.now_data(&m);
            cx.text(
                FontId::Bold,
                t.size_body,
                label,
                Align::Center,
                t.text,
                &format!("Ch {}", d.channel + 1),
            );
            now::paint(cx, &d, &cards);
            return;
        }
        let now = self.clock(&m);
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
        if self.now_shows() {
            // The wheel and - / + pick the channel.
            return match ev {
                IoInput::Wheel(d) | IoInput::Step(d) if d != 0 => self.step_channel(d.signum()),
                _ => IoReply::Ignored,
            };
        }
        match ev {
            IoInput::Wheel(d) => self.scroll(d),
            IoInput::Page(d) => self.scroll(d * self.fit.get() as i32),
            IoInput::Step(_) => IoReply::Ignored,
        }
    }

    fn tick(&mut self, now: Duration) -> IoReply {
        self.now = now;
        let live = {
            let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            // The app's answer to this section's Save log.
            if self.saving {
                if let Some(said) = m.live.log_saved.take() {
                    self.note = Some(said);
                    self.saving = false;
                }
            }
            self.live_now(&m, self.clock(&m))
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

    fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }
}

/// The monitor in debug mode as a page widget: the Dev tab's MIDI page.
/// The same section as the I/O view's Monitor, with the MPC's own
/// controller and MPFree's events among its sources. It draws again
/// whenever what it shows changes.
pub struct DevMonitor {
    s: MonitorSection,
    /// What it showed when last drawn ([`MonitorSection::live_now`]).
    painted: Cell<u64>,
}

impl DevMonitor {
    pub fn new(model: Shared) -> DevMonitor {
        let mut s = MonitorSection::new(model);
        s.debug = true;
        DevMonitor {
            s,
            painted: Cell::new(u64::MAX),
        }
    }

    fn live(&self) -> u64 {
        let m = self.s.model.lock().unwrap_or_else(|e| e.into_inner());
        self.s.live_now(&m, self.s.clock(&m))
    }

    /// The hint line while it shows.
    pub fn hint(&self) -> &'static str {
        DEBUG_HINT
    }
}

impl Widget for DevMonitor {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        self.painted.set(self.live());
        self.s.paint(cx, r);
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        // What a tap changes is the section's own; the page draws it again.
        self.s.touch(finger, phase, x, y, r);
        None
    }

    fn animating(&self) -> bool {
        self.live() != self.painted.get()
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        self.s.targets(r)
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
        let (x, y) = chip(&s, Hit::Chip(0));
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

    /// Save log asks the app for the list as shown; the app's answer comes
    /// to the hint line once. Without a data folder it is greyed and says
    /// why.
    #[test]
    fn save_log_asks_the_app_and_says_its_answer() {
        let mut s = MonitorSection::new(model());
        let (x, y) = chip(&s, Hit::Save);
        tap(&mut s, x, y);
        assert!(s.model.lock().unwrap().asks.save_log.is_none());
        assert!(s.take_note().unwrap().contains("no data folder"));
        s.model.lock().unwrap().live.saves_logs = true;
        // Only what the list shows: the keyboard's notes.
        let (lx, ly) = light(&s, KEYS, Dir::In, 0);
        tap(&mut s, lx, ly);
        tap(&mut s, x, y);
        let ask = s.model.lock().unwrap().asks.save_log.clone().unwrap();
        assert_eq!(ask.lines, 4);
        assert!(
            ask.text.starts_with("MPFree MIDI log, saved "),
            "{}",
            ask.text
        );
        assert!(ask.text.contains("Showing: Keystep 37 in"), "{}", ask.text);
        assert_eq!(ask.stamp.len(), "2026-11-16-1958".len());
        // A second tap before the answer saves nothing more.
        tap(&mut s, x, y);
        assert!(s.take_note().unwrap().contains("still saving"));
        // The app answers; the next tick says it, once.
        {
            let mut m = s.model.lock().unwrap();
            m.asks.save_log = None;
            m.live.log_saved = Some("Saved 4 lines to files/midi-logs/x.txt".into());
        }
        s.tick(Duration::from_secs(3));
        assert_eq!(
            s.take_note().as_deref(),
            Some("Saved 4 lines to files/midi-logs/x.txt")
        );
        s.tick(Duration::from_secs(4));
        assert_eq!(s.take_note(), None);
        assert!(s.model.lock().unwrap().live.log_saved.is_none());
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
            to: To::Unsaid,
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

    #[test]
    fn each_line_says_where_its_message_went() {
        use mpfree_midi::learn::{Mode, Saved, Source, Takeover};
        use mpfree_midi::monitor::Feed;
        use mpfree_param::{OnTrack, ParamAddr};
        let filter = ParamAddr::new(OnTrack::Inst.on(1), 3);
        let mut m = IoModel::default();
        m.live.built.monitor = true;
        m.live.tracks = vec!["Drums".into(), "Bass".into()];
        m.monitor.set_ports(&ports());
        m.setup.learn = vec![Saved {
            port: Some("Keystep 37".into()),
            channel: 0,
            source: Source::Cc(74),
            target: filter,
            lo: 0.0,
            hi: 1.0,
            mode: Mode::Absolute,
            takeover: Takeover::Jump,
        }];
        m.live.learn_names = vec!["Cutoff (Filter, Bass)".into()];
        let mut f = Feed::new();
        let lines: [(&[u8], To); 6] = [
            (
                &[0x99, 37, 100],
                To::Track {
                    track: 0,
                    pad: Some(0),
                },
            ),
            (
                &[0x90, 60, 96],
                To::Track {
                    track: 1,
                    pad: None,
                },
            ),
            (&[0xB0, 74, 88], To::Learned(filter)),
            (&[0xF0, 0x41, 1, 0xF7], To::Ignored),
            (&[0xFA], To::NotFollowed),
            (&[0x99, 20, 90], To::NoPad { track: 0 }),
        ];
        for (b, to) in lines {
            assert!(f.bytes(KEYS, Dir::In, 1.0, b));
            f.to(to);
        }
        f.sent(DIN, 1.1, &[0x91, 36, 110], To::SentBy(1));
        f.sent(DIN, 1.2, &[0xFA], To::Clock);
        f.drain_into(&mut m.monitor, 1.3);
        let went: Vec<(String, Went)> =
            m.monitor.newest().map(|(_, s)| went_to(&m, s.to)).collect();
        let text: Vec<&str> = went.iter().map(|w| w.0.as_str()).collect();
        assert_eq!(
            text,
            [
                "MPFree's clock",
                "sent by Bass",
                "nothing: no pad on Drums",
                "not followed",
                "ignored",
                "Learn: Cutoff (Filter, Bass)",
                "Bass, the armed track",
                "Drums, pad 1",
            ]
        );
        assert_eq!(went[5].1, Went::Learn);
        assert_eq!(went[2].1, Went::Nothing);
        // Learn names a parameter it doesn't know by its address.
        let other = ParamAddr::new(OnTrack::Inst.on(1), 9);
        assert_eq!(
            went_to(&m, To::Took(other)).0,
            format!("Learn took it: {other}")
        );
        assert_eq!(went_to(&m, To::Unsaid).0, "");
        // A route in the Routing section played a track, armed or not.
        assert_eq!(
            went_to(&m, To::Routed { track: 1 }),
            ("Bass, by its route".to_string(), Went::Took)
        );
        // Drawn at the I/O view's width and the Dev page's.
        let s = MonitorSection::new(Arc::new(Mutex::new(m)));
        let _ = paint(&s);
        let _ = cols(1100);
    }

    #[test]
    fn debug_mode_lists_the_mpc_and_mpfrees_events_by_source() {
        use mpfree_midi::monitor::{Event, Feed};
        let model = model();
        let mut f = Feed::new();
        // Off: the debug sources list nothing.
        f.mpc(Dir::In, 3.0, Mpc::KeepAlive);
        f.debug = true;
        f.mpc(
            Dir::In,
            3.0,
            Mpc::Control {
                ev: ControlEvent::Button {
                    note: mpfree_controller::mpc_one::button::PLAY,
                    pressed: true,
                },
                bytes: [0x90, mpfree_controller::mpc_one::button::PLAY, 127],
            },
        );
        f.mpc(
            Dir::Out,
            3.1,
            Mpc::Light {
                button: mpfree_controller::mpc_one::button::PLAY,
                value: 127,
            },
        );
        f.mpc(
            Dir::Out,
            3.2,
            Mpc::Cv {
                out: 0,
                value: 16383,
            },
        );
        f.event(3.3, Event::Plugged(KEYS));
        f.event(
            3.4,
            Event::Unplugged {
                port: KEYS,
                ended: 2,
            },
        );
        f.mpc(Dir::Out, 3.5, Mpc::KeepAlive);
        f.drain_into(&mut model.lock().unwrap().monitor, 3.6);
        let mut d = DevMonitor::new(model);
        let lines = |d: &DevMonitor| -> Vec<String> {
            let m = d.s.model.lock().unwrap();
            d.s.lines(&m.monitor, 100)
                .0
                .iter()
                .map(|(_, l)| {
                    format!(
                        "{} | {}",
                        port_text(&m.monitor, l.port, l.dir),
                        line_words(&m.monitor, l)
                    )
                })
                .collect()
        };
        let all = lines(&d);
        assert_eq!(
            all[..6],
            [
                "MPC out | Keep-alive ping",
                "System | Unplugged: Keystep 37 · 2 held notes let go",
                "System | Plugged in: Keystep 37",
                "MPC out | CV 1 = 16383 · 5.000 V if full scale is 5 V",
                "MPC out | Light PLAY on",
                "MPC in | PLAY pressed",
            ]
        );
        assert_eq!(all.len(), 6 + 6, "and the ports' own: {all:?}");
        // The I/O view's Monitor on the same model keeps to the ports.
        let io = MonitorSection::new(d.s.model.clone());
        assert_eq!(shown(&io).len(), 6);
        // Chips by source: "Ports" hides every port's messages but clock.
        let l = {
            let m = d.s.model.lock().unwrap();
            d.s.layout(&m, AREA)
        };
        let ports = l.chips.iter().find(|c| c.0 == Hit::Chip(0)).unwrap().1;
        let (x, y) = (ports.x + 4, ports.y + 4);
        d.touch(1, Touch::Down, x, y, AREA);
        d.touch(1, Touch::Up, x, y, AREA);
        assert_eq!(
            lines(&d).len(),
            7,
            "the debug lines and the clock: {:?}",
            lines(&d)
        );
        assert!(d.s.targets(AREA).iter().any(|t| t.id == "monitor-source"));
        assert_eq!(d.hint(), DEBUG_HINT);
        // It draws again only when what it shows changes.
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        let theme = Theme::dark();
        let mut fonts = mpfree_ui::Fonts::embedded();
        let mut cx = PaintCx {
            canvas: &mut c,
            theme: &theme,
            fonts: &mut fonts,
        };
        assert!(d.animating());
        d.paint(&mut cx, AREA);
        assert!(!d.animating());
        d.s.model.lock().unwrap().monitor.clear();
        assert!(d.animating());
    }
}
