//! A port's page (board 6 of the MIDI mockups, approved by the owner on
//! 16 Nov 2026): one MIDI port's settings and state on one screen, opened
//! from its row in the MIDI section (a tap on its name, or its Page field).
//!
//! - **This port:** what it is (the 5-pin jacks, or a USB device and which
//!   of its ports) and its **Input** (Plays tracks, Learn only, Both), the
//!   same setting as on its row.
//! - **Ignore coming in:** Clock, Program, Aftertouch and All notes off
//!   ([`IgnoreSet`]). Ignored messages still show in the Monitor, marked
//!   "ignored: this port's setting"; nothing else gets them.
//! - **What it does:** where its messages go today and what plays on it,
//!   read only (the Routing section comes later).
//! - **Now:** the newest message in on it (and out, for the 5-pin jacks),
//!   and how long ago.
//! - **Clock to this port:** Off, While playing or Always: the project's,
//!   the same setting as on the Sync section.
//! - **This gear's delay:** how late the gear on the port sounds. MIDI out
//!   sends that much early, up to the sound's lead ([`Live::early_ms`]).
//! - **Send to it** (SENDS MIDI): a test note and All notes off
//!   ([`ToPort`]), greyed while MIDI out is off and on USB ports until
//!   MPFree sends to USB devices.
//!
//! Input, Ignore coming in and the delay belong to the rig (kept for every
//! project, `MidiSetup::rig`); Clock to this port belongs to the project.
//! The wheel picks a control, - and + step it, and the Q-Links take Input,
//! Clock and the delay. ‹ MIDI or Back closes the page.
//!
//! [`Live::early_ms`]: crate::Live::early_ms

use std::time::Duration;

use mpfree_midi::monitor::{Dir, What};
use mpfree_midi::setup::{
    ClockOut, Ignored, PortInput, PortSetup, TempoSource, DELAY_STEP_TENTHS, MAX_DELAY_TENTHS,
};
use mpfree_mixer_ui::{IoInput, IoKnob, IoReply, IoSection};
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Target, Touch};

use crate::form::Page;
use crate::now::{ago, card};
use crate::{IoModel, PortSend, Shared, ToPort};

const GAP: i32 = 10;
const HEAD_H: i32 = 44;
const BACK_W: i32 = 96;
const VALUE_W: i32 = 168;
const BOX_H: i32 = 40;
const STEP_W: i32 = 44;
const LINE_H: i32 = 19;

/// The hint line while a port's page shows.
pub(crate) const HINT: &str = "Tap a setting to change it  ·  wheel: pick one  ·  - and +: step it  ·  Q-Links: Input, Clock, Delay  ·  ‹ MIDI or Back closes";

/// What the wheel picks, in its order down the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sel {
    Input,
    Ignore(usize),
    Clock,
    Delay,
    TestNote,
    AllOff,
}

const SELS: [Sel; 9] = [
    Sel::Input,
    Sel::Ignore(0),
    Sel::Ignore(1),
    Sel::Ignore(2),
    Sel::Ignore(3),
    Sel::Clock,
    Sel::Delay,
    Sel::TestNote,
    Sel::AllOff,
];

/// What a tap can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Back,
    Input,
    Ignore(usize),
    Clock,
    Delay(i32),
    TestNote,
    AllOff,
}

/// Where everything goes in the section's area.
struct Layout {
    back: Rect,
    head: Rect,
    this: Rect,
    input: Rect,
    ignore: Rect,
    chips: [Rect; 4],
    does: Rect,
    now: Rect,
    clock: Rect,
    clock_box: Rect,
    delay: Rect,
    delay_down: Rect,
    delay_value: Rect,
    delay_up: Rect,
    send: Rect,
    test: Rect,
    all_off: Rect,
}

fn layout(r: Rect) -> Layout {
    let back = Rect::new(r.x, r.y, BACK_W, HEAD_H);
    let head = Rect::new(r.x + BACK_W + 12, r.y, r.w - BACK_W - 12, HEAD_H);
    let body_y = r.y + HEAD_H + GAP;
    let body_h = r.bottom() - body_y;
    let lw = (r.w - GAP) / 2;
    let rx = r.x + lw + GAP;
    let rw = r.w - lw - GAP;
    // Left: this port, ignore coming in, what it does.
    let this = Rect::new(r.x, body_y, lw, 150);
    let ignore = Rect::new(r.x, this.bottom() + GAP, lw, 158);
    let does = Rect::new(
        r.x,
        ignore.bottom() + GAP,
        lw,
        (body_y + body_h - ignore.bottom() - GAP).max(0),
    );
    let inner = |c: Rect| Rect::new(c.x + 14, c.y + 38, c.w - 28, (c.h - 46).max(0));
    let ti = inner(this);
    let input = Rect::new(ti.right() - VALUE_W, ti.y + 26, VALUE_W, BOX_H);
    let ii = inner(ignore);
    let cw = (ii.w - 8) / 2;
    let chips = std::array::from_fn(|k| {
        let (col, row) = ((k % 2) as i32, (k / 2) as i32);
        Rect::new(ii.x + col * (cw + 8), ii.y + row * (BOX_H + 6), cw, BOX_H)
    });
    // Right: now, clock to this port, this gear's delay, send to it.
    let now = Rect::new(rx, body_y, rw, 96);
    let clock = Rect::new(rx, now.bottom() + GAP, rw, 112);
    let ci = inner(clock);
    let clock_box = Rect::new(ci.right() - VALUE_W, ci.y, VALUE_W, BOX_H);
    let delay = Rect::new(rx, clock.bottom() + GAP, rw, 132);
    let di = inner(delay);
    let delay_up = Rect::new(di.right() - STEP_W, di.y, STEP_W, BOX_H);
    let delay_value = Rect::new(delay_up.x - 4 - 84, di.y, 84, BOX_H);
    let delay_down = Rect::new(delay_value.x - 4 - STEP_W, di.y, STEP_W, BOX_H);
    let send = Rect::new(
        rx,
        delay.bottom() + GAP,
        rw,
        (body_y + body_h - delay.bottom() - GAP).max(0),
    );
    let si = inner(send);
    let bw = (si.w - 8) / 2;
    let test = Rect::new(si.x, si.y, bw, 48);
    let all_off = Rect::new(si.x + bw + 8, si.y, si.w - bw - 8, 48);
    Layout {
        back,
        head,
        this,
        input,
        ignore,
        chips,
        does,
        now,
        clock,
        clock_box,
        delay,
        delay_down,
        delay_value,
        delay_up,
        send,
        test,
        all_off,
    }
}

/// Everything the page shows, read from the model once a paint.
struct View {
    name: String,
    setup: PortSetup,
    plugged: bool,
    usb: bool,
    about: String,
    has_in: bool,
    has_out: bool,
    /// MIDI out is on (Prefs).
    out_on: bool,
    early_ms: f32,
    /// The channel the test note goes on: the first track that plays on
    /// this port's, else 1.
    test_channel: u8,
    /// Where its messages go and what plays on it, in words, each with
    /// whether it is in (true) or out.
    does: Vec<(bool, String)>,
    /// The newest message in (and out) on it: way, words, seconds ago.
    last: Vec<(Dir, String, f64)>,
}

impl View {
    /// A test note and All notes off can be sent to it now.
    fn can_send(&self) -> bool {
        self.out_on && !self.usb && self.has_out
    }
}

fn view(m: &IoModel, name: &str, now: f64) -> View {
    let setup = m.setup.port(name);
    let live = m.live.port(name);
    let din = name == mpfree_midi::setup::DIN_NAME;
    let tracks: Vec<(String, u8)> = m
        .setup
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let t = t.as_ref().filter(|t| t.port == name)?;
            let n = m
                .live
                .tracks
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("Track {}", i + 1));
            Some((n, t.channel))
        })
        .collect();
    let leads = m.setup.tempo == TempoSource::Port(name.to_string());
    let mut does = Vec::new();
    does.push((
        true,
        match setup.input {
            PortInput::LearnOnly => "Its notes play nothing (Learn only)".to_string(),
            _ => "Its notes play the armed track".to_string(),
        },
    ));
    if setup.input != PortInput::PlaysTracks {
        let label = if din { "MIDI In" } else { name };
        let learned: Vec<String> = m
            .setup
            .learn
            .iter()
            .filter(|l| l.port.as_deref() == Some(name))
            .map(|l| crate::learn::control_text(&l.source, l.channel, label))
            .map(|t| {
                t.strip_suffix(&format!(" on {label}"))
                    .unwrap_or(&t)
                    .to_string()
            })
            .collect();
        does.push((
            true,
            if learned.is_empty() {
                "Learn: nothing learned from it yet".to_string()
            } else {
                format!("Learn: {}", short_list(&learned, 3))
            },
        ));
    }
    if leads {
        does.push((
            true,
            if setup.ignore.has(Ignored::Clock) {
                "The tempo is from it, but it ignores clock".to_string()
            } else {
                "The tempo follows its clock".to_string()
            },
        ));
    }
    for (n, ch) in tracks.iter().take(3) {
        does.push((false, format!("{n} plays on it · ch {}", ch + 1)));
    }
    if tracks.len() > 3 {
        does.push((false, format!("and {} more tracks", tracks.len() - 3)));
    }
    if setup.clock_out != ClockOut::Off {
        does.push((
            false,
            format!("MPFree's clock: {}", setup.clock_out.label()),
        ));
    }
    let has_out = live.is_none_or(|l| l.has_out);
    if tracks.is_empty() && setup.clock_out == ClockOut::Off {
        does.push((
            false,
            if has_out {
                "Nothing is sent to it".to_string()
            } else {
                "It has no output".to_string()
            },
        ));
    }
    let mon = &m.monitor;
    let id = mon.ports().iter().find(|p| p.name == name).map(|p| p.id);
    let mut last: Vec<(Dir, String, f64)> = Vec::new();
    if let Some(id) = id {
        for dir in [Dir::In, Dir::Out] {
            if dir == Dir::Out && !din {
                continue;
            }
            if let Some((_, s)) = mon
                .newest()
                .find(|(_, s)| s.port == id && s.dir == dir && !matches!(s.what, What::Event(_)))
            {
                let ch = s
                    .what
                    .channel()
                    .map_or(String::new(), |c| format!("ch {} ", c + 1));
                last.push((
                    dir,
                    format!("{ch}{}", crate::monitor_words(s)),
                    (now - s.t).max(0.0),
                ));
            }
        }
    }
    View {
        name: name.to_string(),
        plugged: live.is_some(),
        usb: !din,
        about: live.map_or_else(
            || {
                if din {
                    "The MPC's 5-pin MIDI In and Out jacks".to_string()
                } else {
                    "USB".to_string()
                }
            },
            |l| l.about.clone(),
        ),
        has_in: live.is_none_or(|l| l.has_in),
        has_out,
        out_on: m.live.built.notes_out,
        early_ms: m.live.early_ms,
        test_channel: tracks.first().map_or(0, |t| t.1),
        does,
        last,
        setup,
    }
}

/// "CC 74, CC 71 and 2 more".
fn short_list(v: &[String], most: usize) -> String {
    let mut s = v.iter().take(most).cloned().collect::<Vec<_>>().join(", ");
    if v.len() > most {
        s.push_str(&format!(" and {} more", v.len() - most));
    }
    s
}

/// One port's page.
pub(crate) struct PortPage {
    model: Shared,
    name: String,
    /// The control the wheel picked ([`SELS`]).
    sel: usize,
    done: bool,
    down: Option<(u64, Option<Hit>)>,
    note: Option<String>,
    /// A send was asked: the app's answer is said when it comes.
    waiting: bool,
    now: Duration,
    live: u64,
}

impl PortPage {
    pub fn new(model: Shared, name: &str) -> PortPage {
        PortPage {
            model,
            name: name.to_string(),
            sel: 0,
            done: false,
            down: None,
            note: None,
            waiting: false,
            now: Duration::ZERO,
            live: 0,
        }
    }

    fn view(&self) -> View {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let now = m.monitor.now(self.now.as_secs_f64());
        view(&m, &self.name, now)
    }

    fn hit(&self, x: i32, y: i32, r: Rect) -> Option<Hit> {
        let l = layout(r);
        if l.back.contains(x, y) {
            return Some(Hit::Back);
        }
        if l.input.contains(x, y) {
            return Some(Hit::Input);
        }
        if let Some(k) = l.chips.iter().position(|c| c.contains(x, y)) {
            return Some(Hit::Ignore(k));
        }
        if l.clock_box.contains(x, y) {
            return Some(Hit::Clock);
        }
        if l.delay_down.contains(x, y) {
            return Some(Hit::Delay(-1));
        }
        if l.delay_up.contains(x, y) {
            return Some(Hit::Delay(1));
        }
        if l.test.contains(x, y) {
            return Some(Hit::TestNote);
        }
        if l.all_off.contains(x, y) {
            return Some(Hit::AllOff);
        }
        None
    }

    /// Changes the port's setup; true when something changed.
    fn set(&mut self, f: impl FnOnce(&mut PortSetup)) -> bool {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let before = m.setup.port(&self.name);
        let name = self.name.clone();
        m.setup.set_port(&name, f);
        let changed = m.setup.port(&name) != before;
        if changed {
            m.gen += 1;
        }
        changed
    }

    /// Steps the control `s` by `d` (a tap is +1 on a value, a toggle on a
    /// chip, a press on a button).
    fn step(&mut self, s: Sel, d: i32, tap: bool) -> IoReply {
        if d == 0 {
            return IoReply::Handled;
        }
        let v = self.view();
        let no_in = matches!(s, Sel::Input | Sel::Ignore(_)) && !v.has_in;
        if no_in || (s == Sel::Clock && !v.has_out) {
            self.note = Some(format!(
                "{} has no {}: nothing to set",
                v.name,
                if no_in { "input" } else { "output" }
            ));
            return IoReply::Handled;
        }
        let changed = match s {
            Sel::Input => {
                let c = self.set(|p| p.input = choose(&PortInput::ALL, p.input, d, tap));
                if !c {
                    self.note = Some(at_end("Input", v.setup.input.label(), d));
                }
                c
            }
            Sel::Ignore(k) => {
                let kind = Ignored::ALL[k];
                let on = if tap {
                    !v.setup.ignore.has(kind)
                } else {
                    d > 0
                };
                let c = self.set(|p| p.ignore = p.ignore.with(kind, on));
                if !c {
                    self.note = Some(format!(
                        "{} is {} already: {} {} it",
                        kind.label(),
                        if on { "ignored" } else { "let in" },
                        if on { "-" } else { "+" },
                        if on { "lets in" } else { "ignores" },
                    ));
                }
                c
            }
            Sel::Clock => {
                let c = self.set(|p| p.clock_out = choose(&ClockOut::ALL, p.clock_out, d, tap));
                if !c {
                    self.note = Some(at_end("Clock to this port", v.setup.clock_out.label(), d));
                }
                c
            }
            Sel::Delay => {
                let c = self.set(|p| {
                    let to = i32::from(p.delay_tenths) + d * i32::from(DELAY_STEP_TENTHS);
                    p.delay_tenths = to.clamp(0, i32::from(MAX_DELAY_TENTHS)) as u16;
                });
                if !c {
                    self.note = Some(if d > 0 {
                        format!(
                            "This gear's delay is at its highest, {:.1} ms: - lowers it",
                            f32::from(MAX_DELAY_TENTHS) / 10.0
                        )
                    } else {
                        "This gear's delay is at its lowest, 0.0 ms: + raises it".to_string()
                    });
                }
                c
            }
            Sel::TestNote | Sel::AllOff => {
                if d < 0 {
                    self.note = Some("+ or a tap sends it".to_string());
                    return IoReply::Handled;
                }
                if !v.can_send() {
                    self.note = Some(why_not(&v));
                    return IoReply::Handled;
                }
                let what = if s == Sel::TestNote {
                    ToPort::TestNote {
                        channel: v.test_channel,
                    }
                } else {
                    ToPort::AllNotesOff
                };
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                m.live.port_said = None;
                m.asks.port_send = Some(PortSend {
                    port: self.name.clone(),
                    what,
                });
                self.waiting = true;
                return IoReply::Redraw;
            }
        };
        if changed {
            IoReply::Redraw
        } else {
            IoReply::Handled
        }
    }

    fn tap(&mut self, h: Hit) -> IoReply {
        let sel = match h {
            Hit::Back => {
                self.done = true;
                return IoReply::Redraw;
            }
            Hit::Input => Sel::Input,
            Hit::Ignore(k) => Sel::Ignore(k),
            Hit::Clock => Sel::Clock,
            Hit::Delay(_) => Sel::Delay,
            Hit::TestNote => Sel::TestNote,
            Hit::AllOff => Sel::AllOff,
        };
        self.sel = SELS.iter().position(|s| *s == sel).unwrap_or(0);
        let d = if let Hit::Delay(d) = h { d } else { 1 };
        self.step(sel, d, true).max(IoReply::Redraw)
    }

    fn live_now(&self) -> u64 {
        let v = self.view();
        let mut h = u64::from(v.plugged) | u64::from(v.out_on) << 1;
        for (d, w, s) in &v.last {
            h = h.rotate_left(7)
                ^ (w.len() as u64)
                ^ u64::from(*d == Dir::Out) << 9
                ^ (*s as u64).min(600) << 12
                ^ u64::from(*s < 1.0) << 30;
        }
        h
    }
}

/// The choice `d` steps from `now` in `all`: a tap goes round from the
/// last to the first; - and + and the Q-Links stop at either end, as a
/// knob does.
fn choose<T: Copy + PartialEq>(all: &[T], now: T, d: i32, tap: bool) -> T {
    let n = all.len() as i32;
    let i = all.iter().position(|x| *x == now).unwrap_or(0) as i32;
    let to = if tap {
        (i + d).rem_euclid(n)
    } else {
        (i + d).clamp(0, n - 1)
    };
    all[to as usize]
}

/// What a step past a choice's end says.
fn at_end(what: &str, value: &str, d: i32) -> String {
    if d > 0 {
        format!("{what} is at its last, {value}: - steps back")
    } else {
        format!("{what} is at its first, {value}: + steps on")
    }
}

/// Why a send is greyed, in words.
fn why_not(v: &View) -> String {
    if v.usb {
        "Not sent: MPFree doesn't send to USB devices yet".to_string()
    } else if !v.out_on {
        "Not sent: MIDI out is off (Prefs > Send MIDI and CV out)".to_string()
    } else {
        "Not sent: this port has no output".to_string()
    }
}

/// Lines of small text from `r`'s top, one under the other.
fn lines(cx: &mut PaintCx, r: Rect, text: &[&str], ink: Color) {
    let t = cx.theme.clone();
    for (i, l) in text.iter().enumerate() {
        let lr = Rect::new(r.x, r.y + i as i32 * LINE_H, r.w, LINE_H);
        cx.text(FontId::Sans, t.size_small, lr, Align::Left, ink, l);
    }
}

/// A value box: its text, outlined when picked.
fn value_box(cx: &mut PaintCx, r: Rect, text: &str, picked: bool, enabled: bool) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(r, 8, t.bg);
    if picked {
        cx.canvas.stroke_round_rect(r, 8, 2, t.accent);
    }
    let ink = if enabled { t.text } else { t.text_faint };
    cx.text(FontId::Sans, t.size_body, r, Align::Center, ink, text);
}

/// A button: lit when it can be pressed, outlined when picked.
fn button(cx: &mut PaintCx, r: Rect, text: &str, picked: bool, enabled: bool) {
    let t = cx.theme.clone();
    let (fill, ink) = if enabled {
        (t.raised, t.text)
    } else {
        (t.bg, t.text_faint)
    };
    cx.canvas.fill_round_rect(r, 10, fill);
    if picked {
        cx.canvas.stroke_round_rect(r, 10, 2, t.accent);
    }
    cx.text(FontId::Bold, t.size_small, r, Align::Center, ink, text);
}

fn delay_text(tenths: u16) -> String {
    format!("{:.1} ms", f32::from(tenths) / 10.0)
}

fn test_text(v: &View) -> String {
    format!("Test note C4 · ch {}", v.test_channel + 1)
}

/// What the delay card says under its value.
fn delay_words(v: &View) -> [String; 2] {
    let d = v.setup.delay_ms();
    if v.usb {
        return [
            "Kept for when MPFree sends to".to_string(),
            "USB devices.".to_string(),
        ];
    }
    if !v.out_on {
        return [
            "MIDI out is off (Prefs): kept for".to_string(),
            "when it's on.".to_string(),
        ];
    }
    if v.setup.delay_tenths == 0 {
        return [
            "How late this gear sounds after MIDI".to_string(),
            "reaches it. MPFree sends that much early.".to_string(),
        ];
    }
    if d <= v.early_ms {
        [
            format!("MPFree sends its notes and clock {d:.1} ms"),
            "early, so they land on the beat.".to_string(),
        ]
    } else {
        [
            format!("MPFree sends up to {:.1} ms early today;", v.early_ms),
            "the rest waits for the sequencer.".to_string(),
        ]
    }
}

impl IoSection for PortPage {
    fn label(&self) -> &str {
        crate::MIDI
    }

    fn hint(&self) -> Option<&str> {
        Some(HINT)
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let v = self.view();
        let l = layout(r);
        let picked = SELS[self.sel.min(SELS.len() - 1)];
        // The heading: back, the port's name, plugged in or not.
        button(cx, l.back, "‹ MIDI", false, true);
        cx.text(
            FontId::Bold,
            t.size_title,
            l.head,
            Align::Left,
            t.text,
            &v.name,
        );
        let (pill, ink) = match (v.plugged, v.usb) {
            (true, true) => ("PLUGGED IN · USB", t.good),
            (true, false) => ("5-PIN JACKS", t.good),
            (false, _) => ("NOT PLUGGED IN", t.warn),
        };
        cx.text(FontId::Bold, t.size_small, l.head, Align::Right, ink, pill);

        // This port.
        let body = card(
            cx,
            l.this,
            "THIS PORT",
            "kept for every project",
            t.text_faint,
        );
        lines(cx, body, &[&v.about], t.text_dim);
        let row = Rect::new(body.x, l.input.y, body.w - VALUE_W, BOX_H);
        cx.text(FontId::Sans, t.size_body, row, Align::Left, t.text, "Input");
        value_box(
            cx,
            l.input,
            v.setup.input.label(),
            picked == Sel::Input,
            v.has_in,
        );
        let meaning = match v.setup.input {
            PortInput::PlaysTracks => "Its notes play; Learn doesn't hear it.",
            PortInput::LearnOnly => "Only MIDI Learn hears it; nothing plays.",
            PortInput::Both => "Its notes play, and its controls can be learned.",
        };
        lines(
            cx,
            Rect::new(body.x, l.input.bottom() + 6, body.w, LINE_H),
            &[if v.has_in {
                meaning
            } else {
                "It has no input."
            }],
            t.text_faint,
        );

        // Ignore coming in.
        let body = card(
            cx,
            l.ignore,
            "IGNORE COMING IN",
            "lit = ignored",
            t.text_faint,
        );
        for (k, kind) in Ignored::ALL.iter().enumerate() {
            let on = v.setup.ignore.has(*kind);
            let cr = l.chips[k];
            let (fill, ink) = if on {
                (t.warn, t.on_accent)
            } else {
                (t.raised, t.text_dim)
            };
            cx.canvas.fill_round_rect(cr, 10, fill);
            if picked == Sel::Ignore(k) {
                cx.canvas.stroke_round_rect(cr, 10, 2, t.accent);
            }
            cx.text(
                FontId::Bold,
                t.size_small,
                cr,
                Align::Center,
                ink,
                kind.label(),
            );
        }
        lines(
            cx,
            Rect::new(body.x, l.chips[3].bottom() + 6, body.w, LINE_H),
            &["Ignored messages still show in the Monitor."],
            t.text_faint,
        );

        // What it does.
        let body = card(cx, l.does, "WHAT IT DOES", "", t.text_faint);
        let mut y = body.y;
        for (into, words) in &v.does {
            if y + 24 > body.bottom() - LINE_H {
                break;
            }
            let lr = Rect::new(body.x, y, body.w, 24);
            let arrow = if *into { "→" } else { "←" };
            let ink = if *into { t.good } else { t.accent };
            cx.text(FontId::Bold, t.size_body, lr, Align::Left, ink, arrow);
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(lr.x + 22, lr.y, lr.w - 22, lr.h),
                Align::Left,
                t.text,
                words,
            );
            y += 26;
        }
        lines(
            cx,
            Rect::new(body.x, body.bottom() - LINE_H, body.w, LINE_H),
            &["The Routing section, for any route, comes later."],
            t.text_faint,
        );

        // Now.
        let body = card(cx, l.now, "NOW", "", t.text_faint);
        if !v.plugged {
            lines(
                cx,
                body,
                &["Not plugged in: its settings are kept."],
                t.warn,
            );
        } else if v.last.is_empty() {
            lines(
                cx,
                body,
                &[if v.has_in {
                    "Nothing has come in yet."
                } else {
                    "Nothing has gone out yet."
                }],
                t.text_faint,
            );
        } else {
            for (i, (dir, words, s)) in v.last.iter().enumerate() {
                let lr = Rect::new(body.x, body.y + i as i32 * 24, body.w, 22);
                let way = if *dir == Dir::In { "in " } else { "out" };
                let lit = *s < 1.0;
                cx.text(
                    FontId::Mono,
                    t.size_small,
                    lr,
                    Align::Left,
                    if lit { t.good } else { t.text_dim },
                    &format!("{way} {words}"),
                );
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    lr,
                    Align::Right,
                    t.text_faint,
                    &ago(*s),
                );
            }
        }

        // Clock to this port.
        let body = card(
            cx,
            l.clock,
            "CLOCK TO THIS PORT",
            "saved with the project",
            t.text_faint,
        );
        let row = Rect::new(body.x, l.clock_box.y, body.w - VALUE_W, BOX_H);
        cx.text(
            FontId::Sans,
            t.size_body,
            row,
            Align::Left,
            t.text,
            "Clock, start, stop",
        );
        value_box(
            cx,
            l.clock_box,
            v.setup.clock_out.label(),
            picked == Sel::Clock,
            v.has_out,
        );
        let clock_note = if v.usb {
            "Kept: MPFree doesn't send to USB devices yet."
        } else if !v.out_on {
            "Kept: MIDI out is off (Prefs)."
        } else {
            "The same setting as on the Sync section."
        };
        lines(
            cx,
            Rect::new(body.x, l.clock_box.bottom() + 6, body.w, LINE_H),
            &[clock_note],
            t.text_faint,
        );

        // This gear's delay.
        let body = card(
            cx,
            l.delay,
            "THIS GEAR'S DELAY",
            "kept for every project",
            t.text_faint,
        );
        let row = Rect::new(body.x, l.delay_down.y, l.delay_down.x - body.x, BOX_H);
        cx.text(
            FontId::Sans,
            t.size_body,
            row,
            Align::Left,
            t.text,
            "It sounds late by",
        );
        button(cx, l.delay_down, "−", false, v.setup.delay_tenths > 0);
        value_box(
            cx,
            l.delay_value,
            &delay_text(v.setup.delay_tenths),
            picked == Sel::Delay,
            true,
        );
        button(
            cx,
            l.delay_up,
            "+",
            false,
            v.setup.delay_tenths < MAX_DELAY_TENTHS,
        );
        let words = delay_words(&v);
        lines(
            cx,
            Rect::new(body.x, l.delay_down.bottom() + 6, body.w, LINE_H * 2),
            &[&words[0], &words[1]],
            t.text_faint,
        );

        // Send to it.
        let body = card(cx, l.send, "SEND TO IT", "SENDS MIDI", t.warn);
        let can = v.can_send();
        button(cx, l.test, &test_text(&v), picked == Sel::TestNote, can);
        button(cx, l.all_off, "All notes off", picked == Sel::AllOff, can);
        let send_note: [&str; 2] = if can {
            [
                "All notes off ends MPFree's notes, then",
                "sends CC 123 on all 16 channels.",
            ]
        } else if v.usb {
            ["Greyed: MPFree doesn't send to USB", "devices yet."]
        } else {
            [
                "Greyed while MIDI out is off (Prefs >",
                "Send MIDI and CV out).",
            ]
        };
        lines(
            cx,
            Rect::new(body.x, l.test.bottom() + 8, body.w, LINE_H * 2),
            &send_note,
            t.text_faint,
        );
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let v = self.view();
        let l = layout(r);
        let mut out = vec![
            Target::tap("port-back", 0, "‹ MIDI", l.back),
            Target::tap(
                "port-input",
                0,
                format!("Input: {}", v.setup.input.label()),
                l.input,
            )
            .disabled_if(!v.has_in),
        ];
        for (k, kind) in Ignored::ALL.iter().enumerate() {
            out.push(
                Target::tap("port-ignore", k, kind.label(), l.chips[k]).disabled_if(!v.has_in),
            );
        }
        out.push(
            Target::tap(
                "port-clock",
                0,
                format!("Clock: {}", v.setup.clock_out.label()),
                l.clock_box,
            )
            .disabled_if(!v.has_out),
        );
        out.push(
            Target::tap("port-delay", 0, "Delay −", l.delay_down)
                .disabled_if(v.setup.delay_tenths == 0),
        );
        out.push(
            Target::tap("port-delay", 1, "Delay +", l.delay_up)
                .disabled_if(v.setup.delay_tenths >= MAX_DELAY_TENTHS),
        );
        out.push(
            Target::tap("port-test-note", 0, test_text(&v), l.test).disabled_if(!v.can_send()),
        );
        out.push(
            Target::tap("port-all-notes-off", 0, "All notes off", l.all_off)
                .disabled_if(!v.can_send()),
        );
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
        match ev {
            IoInput::Wheel(d) | IoInput::Page(d) => {
                let to = (self.sel as i32 + d).clamp(0, SELS.len() as i32 - 1) as usize;
                if to == self.sel {
                    self.note = Some(
                        if d > 0 {
                            "The last setting is picked"
                        } else {
                            "The first setting is picked"
                        }
                        .to_string(),
                    );
                    return IoReply::Handled;
                }
                self.sel = to;
                IoReply::Redraw
            }
            IoInput::Step(d) => self.step(SELS[self.sel], d, false),
        }
    }

    fn tick(&mut self, now: Duration) -> IoReply {
        self.now = now;
        let mut reply = IoReply::Ignored;
        if self.waiting {
            let said = {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                m.live.port_said.take()
            };
            if let Some(s) = said {
                self.note = Some(s);
                self.waiting = false;
                reply = IoReply::Redraw;
            }
        }
        let live = self.live_now();
        if live != self.live {
            self.live = live;
            reply = IoReply::Redraw;
        }
        reply
    }

    fn back(&mut self) -> bool {
        false
    }

    fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    fn qlinks(&self) -> Option<[IoKnob; 4]> {
        let v = self.view();
        let pos = |i: usize, n: usize| i as f32 / (n.max(2) - 1) as f32;
        let input = PortInput::ALL
            .iter()
            .position(|x| *x == v.setup.input)
            .unwrap_or(0);
        let clock = ClockOut::ALL
            .iter()
            .position(|x| *x == v.setup.clock_out)
            .unwrap_or(0);
        Some([
            IoKnob {
                label: "Input".to_string(),
                text: v.setup.input.label().to_string(),
                pos: pos(input, PortInput::ALL.len()),
                enabled: v.has_in,
            },
            IoKnob {
                label: "Clock".to_string(),
                text: v.setup.clock_out.label().to_string(),
                pos: pos(clock, ClockOut::ALL.len()),
                enabled: v.has_out,
            },
            IoKnob {
                label: "Delay".to_string(),
                text: delay_text(v.setup.delay_tenths),
                pos: f32::from(v.setup.delay_tenths) / f32::from(MAX_DELAY_TENTHS),
                enabled: true,
            },
            IoKnob {
                label: String::new(),
                text: String::new(),
                pos: 0.0,
                enabled: false,
            },
        ])
    }

    fn qlink_turn(&mut self, knob: usize, detents: i32, fine: bool) -> IoReply {
        if detents == 0 {
            return IoReply::Ignored;
        }
        let d = if fine { detents.signum() } else { detents };
        match knob {
            0 => self.step(Sel::Input, d.signum(), false),
            1 => self.step(Sel::Clock, d.signum(), false),
            2 => self.step(Sel::Delay, d, false),
            _ => IoReply::Ignored,
        }
    }
}

impl Page for PortPage {
    fn done(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::testing::{paint, tap, AREA};
    use mpfree_midi::setup::DIN_NAME;
    use std::sync::{Arc, Mutex};

    fn model() -> Shared {
        Arc::new(Mutex::new(crate::sample::rig()))
    }

    fn centre(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    fn port(m: &Shared, name: &str) -> PortSetup {
        m.lock().unwrap().setup.port(name)
    }

    #[test]
    fn every_control_fits_and_says_what_it_is() {
        let m = model();
        for name in [DIN_NAME, "Keystep 37"] {
            let p = PortPage::new(m.clone(), name);
            let _ = paint(&p);
            let ts = p.targets(AREA);
            assert_eq!(ts.len(), 11, "{name}");
            for t in ts {
                let r = t.rect;
                assert!(
                    r.x >= AREA.x
                        && r.y >= AREA.y
                        && r.right() <= AREA.right()
                        && r.y + r.h <= AREA.y + AREA.h,
                    "{name}: {} \"{}\" at {r:?} is outside {AREA:?}",
                    t.id,
                    t.label
                );
            }
        }
    }

    /// Taps and the wheel set the rig's settings for the port; a change
    /// moves the model's generation, so the app applies and keeps it.
    #[test]
    fn the_page_sets_the_ports_input_ignores_clock_and_delay() {
        let m = model();
        let mut p = PortPage::new(m.clone(), "Keystep 37");
        let l = layout(AREA);
        let gen = m.lock().unwrap().gen;
        let before = port(&m, "Keystep 37").input;
        let (x, y) = centre(l.input);
        assert_eq!(tap(&mut p, x, y), IoReply::Redraw);
        assert_ne!(port(&m, "Keystep 37").input, before);
        assert!(m.lock().unwrap().gen > gen);
        // Two chips on, one off again.
        for k in [1, 3, 1] {
            let (x, y) = centre(l.chips[k]);
            tap(&mut p, x, y);
        }
        let ig = port(&m, "Keystep 37").ignore;
        assert!(ig.has(Ignored::AllNotesOff) && !ig.has(Ignored::Program));
        assert!(!ig.has(Ignored::Clock));
        // The delay: + three times, - once, by the buttons.
        for d in [l.delay_up, l.delay_up, l.delay_up, l.delay_down] {
            let (x, y) = centre(d);
            tap(&mut p, x, y);
        }
        assert_eq!(port(&m, "Keystep 37").delay_tenths, 2 * DELAY_STEP_TENTHS);
        // The wheel picks Clock, + steps it.
        p.sel = 0;
        p.input(IoInput::Wheel(5));
        assert_eq!(SELS[p.sel], Sel::Clock);
        let was = port(&m, "Keystep 37").clock_out;
        p.input(IoInput::Step(1));
        assert_ne!(port(&m, "Keystep 37").clock_out, was);
        // The Q-Links: the delay at its lowest says so.
        p.qlink_turn(2, -40, false);
        assert_eq!(port(&m, "Keystep 37").delay_tenths, 0);
        assert_eq!(p.qlink_turn(2, -1, false), IoReply::Handled);
        assert!(p.take_note().unwrap().contains("at its lowest"));
        // Q-Link 1 stops at Input's last choice, as a knob does.
        for _ in 0..3 {
            p.qlink_turn(0, 1, false);
        }
        assert_eq!(port(&m, "Keystep 37").input, PortInput::ALL[2]);
        assert_eq!(p.qlink_turn(0, 1, false), IoReply::Handled);
        assert!(p.take_note().unwrap().contains("Input is at its last"));
        // The rig keeps the input, ignores and delay, not the clock.
        let rig = m.lock().unwrap().setup.rig();
        assert_eq!(rig.port("Keystep 37").ignore, ig);
        // ‹ MIDI asks to close.
        let (x, y) = centre(l.back);
        tap(&mut p, x, y);
        assert!(p.done());
    }

    /// The sends ask the app only while MIDI out is on and only for the
    /// 5-pin jacks, and say the app's answer.
    #[test]
    fn sends_ask_the_app_and_say_its_answer() {
        let m = model();
        let l = layout(AREA);
        let mut p = PortPage::new(m.clone(), DIN_NAME);
        m.lock().unwrap().live.built.notes_out = false;
        let (x, y) = centre(l.test);
        tap(&mut p, x, y);
        assert_eq!(m.lock().unwrap().asks.port_send, None);
        assert!(p.take_note().unwrap().contains("MIDI out is off"));
        m.lock().unwrap().live.built.notes_out = true;
        tap(&mut p, x, y);
        let ask = m.lock().unwrap().asks.port_send.clone().unwrap();
        assert_eq!(ask.port, DIN_NAME);
        // The sample rig's first track on the jacks plays on channel 10.
        let ch = m
            .lock()
            .unwrap()
            .setup
            .tracks
            .iter()
            .flatten()
            .find(|t| t.port == DIN_NAME)
            .map(|t| t.channel)
            .unwrap_or(0);
        assert_eq!(ask.what, ToPort::TestNote { channel: ch });
        m.lock().unwrap().live.port_said = Some("Test note C4 sent".to_string());
        assert_eq!(p.tick(Duration::from_secs(1)), IoReply::Redraw);
        assert_eq!(p.take_note().as_deref(), Some("Test note C4 sent"));
        let (x, y) = centre(l.all_off);
        tap(&mut p, x, y);
        assert_eq!(
            m.lock().unwrap().asks.port_send.as_ref().map(|a| a.what),
            Some(ToPort::AllNotesOff)
        );
        // A USB port sends nothing yet.
        let mut usb = PortPage::new(m.clone(), "Keystep 37");
        m.lock().unwrap().asks.port_send = None;
        let (x, y) = centre(l.test);
        tap(&mut usb, x, y);
        assert_eq!(m.lock().unwrap().asks.port_send, None);
        assert!(usb.take_note().unwrap().contains("USB"));
    }

    #[test]
    fn what_it_does_names_learned_controls_and_tracks() {
        let m = model();
        let v = view(&m.lock().unwrap(), DIN_NAME, 0.0);
        let words: Vec<&str> = v.does.iter().map(|d| d.1.as_str()).collect();
        assert!(words.iter().any(|w| w.contains("plays on it")), "{words:?}");
        let v = view(&m.lock().unwrap(), "Keystep 37", 0.0);
        assert!(
            v.does.iter().any(|d| d.1.starts_with("Learn:")),
            "{:?}",
            v.does
        );
    }

    #[test]
    fn the_delay_card_says_how_early_mpfree_sends() {
        let m = model();
        {
            let mut g = m.lock().unwrap();
            g.live.built.notes_out = true;
            g.live.early_ms = 8.7;
            g.setup.set_port(DIN_NAME, |p| p.delay_tenths = 60);
        }
        let v = view(&m.lock().unwrap(), DIN_NAME, 0.0);
        assert!(delay_words(&v)[0].contains("6.0 ms"));
        m.lock()
            .unwrap()
            .setup
            .set_port(DIN_NAME, |p| p.delay_tenths = 200);
        let v = view(&m.lock().unwrap(), DIN_NAME, 0.0);
        assert!(delay_words(&v)[0].contains("up to 8.7 ms"));
    }
}
