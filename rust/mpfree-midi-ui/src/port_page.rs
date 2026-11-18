//! A port's page (board 6 of the MIDI mockups, approved by the owner on
//! 16 Nov 2026): one MIDI port's settings and state on one screen, opened
//! from its row in the MIDI section (a tap on its name, or its Page field).
//!
//! - **This port:** its **Name** (a USB port's, typed on the text keyboard
//!   through [`Asks::name_port`]: "Juno" for a "USB MIDI Interface", shown
//!   on every screen; the 5-pin jacks keep the panel's MIDI In and MIDI
//!   Out), what it is (the 5-pin jacks, or a USB device and which of its
//!   ports, and the name it calls itself), its **Input** (Plays tracks,
//!   Learn only, Both), the same setting as on its row, and its
//!   **Profile** ([`mpfree_midi::profile`]): Auto (the first profile made
//!   for the name it calls itself), None, or one by name, which names its
//!   CCs and programs in the Monitor and Learn.
//! - **Ignore coming in:** Clock, Program, Aftertouch and All notes off
//!   ([`IgnoreSet`]). Ignored messages still show in the Monitor, marked
//!   "ignored: this port's setting"; nothing else gets them.
//! - **What it does:** where its messages go today and what plays on it,
//!   read only (the Routing section changes it): its routes to tracks and
//!   its thru to outputs, then where the rest goes; what plays on it, thru
//!   from other ports included.
//! - **Now:** the newest message in on it and out to it, and how long
//!   ago.
//! - **Clock to this port:** Off, While playing or Always: the project's,
//!   the same setting as on the Sync section.
//! - **This gear's delay:** how late the gear on the port sounds. MIDI out
//!   sends that much early, up to the sound's lead ([`Live::early_ms`]).
//! - **Send to it** (SENDS MIDI): a test note and All notes off
//!   ([`ToPort`]), greyed unless MIDI out sends to the port now
//!   ([`PortLive::sending`]: MIDI out on, the port plugged in and its
//!   output open).
//!
//! The name, Input, Profile, Ignore coming in and the delay belong to the rig (kept
//! for every project, `MidiSetup::rig`); Clock to this port belongs to the
//! project.
//! The wheel picks a control, - and + step it, and the Q-Links take Input,
//! Clock and the delay. ‹ MIDI or Back closes the page.
//!
//! [`Asks::name_port`]: crate::Asks::name_port
//! [`Live::early_ms`]: crate::Live::early_ms
//! [`PortLive::sending`]: crate::PortLive::sending

use std::time::Duration;

use mpfree_midi::monitor::{Dir, What};
use mpfree_midi::setup::{
    ClockOut, Ignored, PortInput, PortSetup, ProfilePick, TempoSource, DELAY_STEP_TENTHS,
    MAX_CLOCK_DIVIDE, MAX_DELAY_TENTHS, MAX_NUDGE_TENTHS, NUDGE_STEP_TENTHS,
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
/// The Name row's word, left of its box.
const NAME_W: i32 = 72;
/// The narrowest Ignore chip in a row of four, for "All notes off" in
/// the smaller letters; from [`CHIP_W`] the usual ones.
const CHIP_MIN_W: i32 = 80;
const CHIP_W: i32 = 96;
/// The clock card's Divide and Nudge, side by side: their labels, - and
/// + and values.
const DIVIDE_LABEL_W: i32 = 54;
const NUDGE_LABEL_W: i32 = 56;
const SMALL_STEP_W: i32 = 30;
const DIVIDE_W: i32 = 30;
const NUDGE_W: i32 = 66;
/// Divide and Nudge fit on one line from this card width in: their
/// labels, values and four steps, the four gaps inside the steppers,
/// Nudge's gap after its label and at least as much between the two.
const CLOCK_ROW_W: i32 =
    DIVIDE_LABEL_W + DIVIDE_W + NUDGE_LABEL_W + NUDGE_W + 4 * SMALL_STEP_W + 6 * 4;

/// The hint line while a port's page shows.
pub(crate) const HINT: &str = "Tap a setting to change it (Name: type one)  ·  wheel: pick one  ·  - and +: step it  ·  Q-Links: Input, Clock, Delay, Divide  ·  ‹ MIDI or Back closes";

/// What the wheel picks, in its order down the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sel {
    Name,
    Input,
    Profile,
    Ignore(usize),
    Clock,
    Divide,
    Nudge,
    Delay,
    TestNote,
    AllOff,
}

const SELS: [Sel; 13] = [
    Sel::Name,
    Sel::Input,
    Sel::Profile,
    Sel::Ignore(0),
    Sel::Ignore(1),
    Sel::Ignore(2),
    Sel::Ignore(3),
    Sel::Clock,
    Sel::Divide,
    Sel::Nudge,
    Sel::Delay,
    Sel::TestNote,
    Sel::AllOff,
];

/// What a tap can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Back,
    Name,
    Input,
    Profile,
    Ignore(usize),
    Clock,
    Divide(i32),
    Nudge(i32),
    Delay(i32),
    TestNote,
    AllOff,
}

/// Where everything goes in the section's area.
struct Layout {
    back: Rect,
    head: Rect,
    this: Rect,
    name: Rect,
    input: Rect,
    profile: Rect,
    ignore: Rect,
    chips: [Rect; 4],
    /// The chips are in one row (a wide page), with Ignore's note in its
    /// heading, leaving What it does room for five lines.
    one_row: bool,
    does: Rect,
    now: Rect,
    clock: Rect,
    clock_box: Rect,
    /// Divide's and Nudge's labels and their -, value and +: on one line
    /// in a wide card, else a line each.
    divide_label: Rect,
    divide: [Rect; 3],
    nudge_label: Rect,
    nudge: [Rect; 3],
    /// The clock card's last line: what was measured, or why it waits.
    clock_line: Rect,
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
    let this = Rect::new(r.x, body_y, lw, 256);
    // Four chips in a row where each has room for "All notes off".
    let one_row = lw - 28 >= 4 * CHIP_MIN_W + 3 * 8;
    let ignore_h = if one_row { 38 + BOX_H + 12 } else { 158 };
    let ignore = Rect::new(r.x, this.bottom() + GAP, lw, ignore_h);
    let does = Rect::new(
        r.x,
        ignore.bottom() + GAP,
        lw,
        (body_y + body_h - ignore.bottom() - GAP).max(0),
    );
    let inner = |c: Rect| Rect::new(c.x + 14, c.y + 38, c.w - 28, (c.h - 46).max(0));
    let ti = inner(this);
    let name = Rect::new(ti.x + NAME_W, ti.y, ti.w - NAME_W, BOX_H);
    let input = Rect::new(ti.right() - VALUE_W, ti.y + BOX_H + 8, VALUE_W, BOX_H);
    // Under Input's line: its profile, as wide as the name.
    let profile = Rect::new(
        ti.x + NAME_W,
        input.bottom() + 6 + LINE_H + 8,
        ti.w - NAME_W,
        BOX_H,
    );
    let ii = inner(ignore);
    let per_row = if one_row { 4 } else { 2 };
    let cw = (ii.w - 8 * (per_row - 1)) / per_row;
    let chips = std::array::from_fn(|k| {
        let (col, row) = ((k as i32 % per_row), (k as i32 / per_row));
        Rect::new(ii.x + col * (cw + 8), ii.y + row * (BOX_H + 6), cw, BOX_H)
    });
    // Right: now, clock to this port, this gear's delay, send to it.
    let now = Rect::new(rx, body_y, rw, 84);
    let one_line = rw - 28 >= CLOCK_ROW_W;
    let rows = if one_line { 2 } else { 3 };
    let clock_h = 38 + rows * BOX_H + (rows - 1) * 5 + 6 + LINE_H + 8;
    let clock = Rect::new(rx, now.bottom() + GAP, rw, clock_h);
    let ci = inner(clock);
    let clock_box = Rect::new(ci.right() - VALUE_W, ci.y, VALUE_W, BOX_H);
    // -, value, + ending at `right`.
    let stepper = |right: i32, y: i32, value_w: i32, step_w: i32| {
        let up = Rect::new(right - step_w, y, step_w, BOX_H);
        let value = Rect::new(up.x - 4 - value_w, y, value_w, BOX_H);
        let down = Rect::new(value.x - 4 - step_w, y, step_w, BOX_H);
        [down, value, up]
    };
    let row1 = clock_box.bottom() + 5;
    let (divide_label, divide, nudge_label, nudge) = if one_line {
        let divide = stepper(
            ci.x + DIVIDE_LABEL_W + 2 * SMALL_STEP_W + DIVIDE_W + 8,
            row1,
            DIVIDE_W,
            SMALL_STEP_W,
        );
        let nudge = stepper(ci.right(), row1, NUDGE_W, SMALL_STEP_W);
        (
            Rect::new(ci.x, row1, DIVIDE_LABEL_W, BOX_H),
            divide,
            Rect::new(nudge[0].x - 4 - NUDGE_LABEL_W, row1, NUDGE_LABEL_W, BOX_H),
            nudge,
        )
    } else {
        let row2 = row1 + BOX_H + 5;
        let divide = stepper(ci.right(), row1, NUDGE_W, STEP_W);
        let nudge = stepper(ci.right(), row2, NUDGE_W, STEP_W);
        (
            Rect::new(ci.x, row1, divide[0].x - ci.x, BOX_H),
            divide,
            Rect::new(ci.x, row2, nudge[0].x - ci.x, BOX_H),
            nudge,
        )
    };
    let clock_line = Rect::new(ci.x, nudge[0].bottom() + 6, ci.w, LINE_H);
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
        name,
        input,
        profile,
        ignore,
        chips,
        one_row,
        does,
        now,
        clock,
        clock_box,
        divide_label,
        divide,
        nudge_label,
        nudge,
        clock_line,
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
    /// Its name on screen ([`mpfree_midi::setup::MidiSetup::shown`]).
    shown: String,
    setup: PortSetup,
    /// The profiles' names, in the order - and + step through them.
    profiles: Vec<String>,
    /// Profile's value: "Auto · Juno-X", "None", "General MIDI".
    profile_label: String,
    /// What the profile names, in words, under it.
    profile_says: String,
    plugged: bool,
    usb: bool,
    about: String,
    has_in: bool,
    has_out: bool,
    /// MIDI out is on (Prefs).
    out_on: bool,
    /// MIDI out sends to it now.
    sending: bool,
    /// Its clock as it left: the gap between pulses and their spread
    /// either side, in ms ([`crate::PortLive::clock_measured`]).
    measured: Option<(f32, f32)>,
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
        self.sending
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
    let routes: Vec<String> = m
        .setup
        .routes
        .iter()
        .filter(|r| r.port == name)
        .map(|r| {
            let ch = r
                .channel
                .map_or("All channels".to_string(), |c| format!("Ch {}", c + 1));
            let t = m
                .live
                .tracks
                .get(usize::from(r.track))
                .cloned()
                .unwrap_or_else(|| format!("Track {}", u16::from(r.track) + 1));
            format!("{ch} → {t}")
        })
        .collect();
    let out_name = |p: &str| {
        if p == mpfree_midi::setup::DIN_NAME {
            "MIDI out".to_string()
        } else {
            m.setup.shown(p).to_string()
        }
    };
    let routes: Vec<String> = routes
        .into_iter()
        .chain(m.setup.thru.iter().filter(|r| r.from == name).map(|r| {
            let ch = r
                .channel
                .map_or("All channels".to_string(), |c| format!("Ch {}", c + 1));
            format!("{ch} → {}, thru", out_name(&r.to))
        }))
        .collect();
    let thru_in: Vec<String> = m
        .setup
        .thru
        .iter()
        .filter(|r| r.to == name)
        .map(|r| {
            let from = if r.from == mpfree_midi::setup::DIN_NAME {
                "MIDI in"
            } else {
                m.setup.shown(&r.from)
            };
            match r.channel {
                Some(c) => format!("{from} · ch {}", c + 1),
                None => from.to_string(),
            }
        })
        .collect();
    let mut does = Vec::new();
    if !routes.is_empty() {
        does.push((true, short_list(&routes, 3)));
    }
    let rest = if routes.is_empty() {
        "Its notes"
    } else {
        "The rest"
    };
    does.push((
        true,
        match setup.input {
            PortInput::LearnOnly => format!("{rest} play nothing (Learn only)"),
            _ => format!("{rest} play the armed track"),
        },
    ));
    if setup.input != PortInput::PlaysTracks {
        let label = if din { "MIDI In" } else { m.setup.shown(name) };
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
    if !thru_in.is_empty() {
        does.push((false, format!("Thru from {}", short_list(&thru_in, 2))));
    }
    if setup.clock_out != ClockOut::Off {
        does.push((
            false,
            if setup.clock_divide > 1 {
                format!(
                    "MPFree's clock: {}, divided by {}",
                    setup.clock_out.label(),
                    setup.clock_divide
                )
            } else {
                format!("MPFree's clock: {}", setup.clock_out.label())
            },
        ));
    }
    let has_out = live.is_none_or(|l| l.has_out);
    if tracks.is_empty() && thru_in.is_empty() && setup.clock_out == ClockOut::Off {
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
                    format!("{ch}{}", crate::monitor_words_named(s, m.profile(name))),
                    (now - s.t).max(0.0),
                ));
            }
        }
    }
    View {
        name: name.to_string(),
        shown: m.setup.shown(name).to_string(),
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
        sending: live.is_some_and(|l| l.sending),
        measured: live.and_then(|l| l.clock_measured),
        early_ms: m.live.early_ms,
        test_channel: tracks.first().map_or(0, |t| t.1),
        does,
        last,
        profiles: m.profiles.all().iter().map(|p| p.name.clone()).collect(),
        profile_label: profile_label(m, name, &setup.profile),
        profile_says: profile_says(m.profile(name), &setup.profile),
        setup,
    }
}

/// Profile's value on the page.
fn profile_label(m: &IoModel, name: &str, pick: &ProfilePick) -> String {
    match pick {
        ProfilePick::Auto => match m.profiles.matching(name) {
            Some(p) => format!("Auto · {}", p.name),
            None => "Auto · none made for it".to_string(),
        },
        ProfilePick::Off => "None".to_string(),
        ProfilePick::Named(n) => match m.profiles.get(n) {
            Some(p) => p.name.clone(),
            None => format!("{n} (not found)"),
        },
    }
}

/// The line under Profile: what the port's profile names.
fn profile_says(p: Option<&mpfree_midi::profile::Profile>, pick: &ProfilePick) -> String {
    let Some(p) = p else {
        return match pick {
            // Its name shows in the box above, with "(not found)".
            ProfilePick::Named(_) => "Not found: usual names for controls.".to_string(),
            _ => "Usual names for controls (CC 7 Volume).".to_string(),
        };
    };
    let (cc, nrpn, programs) = p.counts();
    let mut parts = Vec::new();
    for (n, what) in [(cc, "control"), (nrpn, "NRPN"), (programs, "program")] {
        match n {
            0 => {}
            1 => parts.push(format!("1 {what}")),
            n => parts.push(format!("{n} {what}s")),
        }
    }
    // A drum machine's: its kit pads too, in a shorter list that fits.
    if p.pads() > 0 {
        let mut short = Vec::new();
        for (n, one, many) in [
            (cc, "CC", "CCs"),
            (nrpn, "NRPN", "NRPNs"),
            (programs, "program", "programs"),
            (p.pads(), "kit pad", "pads"),
        ] {
            match n {
                0 => {}
                1 => short.push(format!("1 {one}")),
                n => short.push(format!("{n} {many}")),
            }
        }
        return format!("Covers {}.", short.join(", "));
    }
    let Some(last) = parts.pop() else {
        return "It names nothing: usual names for controls.".to_string();
    };
    let list = if parts.is_empty() {
        last
    } else {
        format!("{} and {last}", parts.join(", "))
    };
    format!("Names {list}.")
}

/// The profile choice `d` steps to from `now`: Auto, None, then each
/// profile by name. A tap goes round; - and + stop at either end. A
/// profile no longer there steps from Auto.
fn step_profile(names: &[String], now: &ProfilePick, d: i32, tap: bool) -> ProfilePick {
    let all: Vec<ProfilePick> = [ProfilePick::Auto, ProfilePick::Off]
        .into_iter()
        .chain(names.iter().map(|n| ProfilePick::Named(n.clone())))
        .collect();
    let n = all.len() as i32;
    let i = all.iter().position(|x| x == now).unwrap_or(0) as i32;
    let to = if tap {
        (i + d).rem_euclid(n)
    } else {
        (i + d).clamp(0, n - 1)
    };
    all[to as usize].clone()
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
        if l.name.contains(x, y) {
            return Some(Hit::Name);
        }
        if l.input.contains(x, y) {
            return Some(Hit::Input);
        }
        if l.profile.contains(x, y) {
            return Some(Hit::Profile);
        }
        if let Some(k) = l.chips.iter().position(|c| c.contains(x, y)) {
            return Some(Hit::Ignore(k));
        }
        if l.clock_box.contains(x, y) {
            return Some(Hit::Clock);
        }
        for (rr, d) in [(l.divide[0], -1), (l.divide[2], 1)] {
            if rr.contains(x, y) {
                return Some(Hit::Divide(d));
            }
        }
        for (rr, d) in [(l.nudge[0], -1), (l.nudge[2], 1)] {
            if rr.contains(x, y) {
                return Some(Hit::Nudge(d));
            }
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
        if s == Sel::Name {
            if !v.usb {
                self.note = Some(
                    "The 5-pin jacks keep the names on the MPC's panel: MIDI In and MIDI Out"
                        .to_string(),
                );
                return IoReply::Handled;
            }
            if d < 0 {
                self.note = Some("+ or a tap types its name".to_string());
                return IoReply::Handled;
            }
            let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            m.asks.name_port = Some(self.name.clone());
            return IoReply::Redraw;
        }
        let no_in = matches!(s, Sel::Input | Sel::Ignore(_)) && !v.has_in;
        let clock = matches!(s, Sel::Clock | Sel::Divide | Sel::Nudge);
        if no_in || (clock && !v.has_out) {
            self.note = Some(format!(
                "{} has no {}: nothing to set",
                v.shown,
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
            Sel::Profile => {
                let to = step_profile(&v.profiles, &v.setup.profile, d, tap);
                let c = self.set(|p| p.profile = to);
                if !c {
                    self.note = Some(at_end("Profile", &v.profile_label, d));
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
            Sel::Divide => {
                let c = self.set(|p| {
                    let to = i32::from(p.clock_divide) + d;
                    p.clock_divide = to.clamp(1, i32::from(MAX_CLOCK_DIVIDE)) as u8;
                });
                if !c {
                    self.note = Some(if d > 0 {
                        format!("Divide is at its most, {MAX_CLOCK_DIVIDE}: - speeds the clock up")
                    } else {
                        "Divide is at 1, the tempo itself: + slows the clock down".to_string()
                    });
                }
                c
            }
            Sel::Nudge => {
                let c = self.set(|p| {
                    let to = i32::from(p.clock_nudge_tenths) + d * i32::from(NUDGE_STEP_TENTHS);
                    let max = i32::from(MAX_NUDGE_TENTHS);
                    p.clock_nudge_tenths = to.clamp(-max, max) as i16;
                });
                if !c {
                    let max = f32::from(MAX_NUDGE_TENTHS) / 10.0;
                    self.note = Some(if d > 0 {
                        format!("The clock's nudge is at its latest, +{max:.1} ms")
                    } else {
                        format!("The clock's nudge is at its earliest, -{max:.1} ms")
                    });
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
            // Asked of the app above.
            Sel::Name => false,
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
            Hit::Name => Sel::Name,
            Hit::Input => Sel::Input,
            Hit::Profile => Sel::Profile,
            Hit::Ignore(k) => Sel::Ignore(k),
            Hit::Clock => Sel::Clock,
            Hit::Divide(_) => Sel::Divide,
            Hit::Nudge(_) => Sel::Nudge,
            Hit::Delay(_) => Sel::Delay,
            Hit::TestNote => Sel::TestNote,
            Hit::AllOff => Sel::AllOff,
        };
        self.sel = SELS.iter().position(|s| *s == sel).unwrap_or(0);
        let d = match h {
            Hit::Delay(d) | Hit::Divide(d) | Hit::Nudge(d) => d,
            _ => 1,
        };
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
    if !v.out_on {
        "Not sent: MIDI out is off (Prefs > Send MIDI and CV out)".to_string()
    } else if !v.plugged {
        format!("Not sent: {} is not plugged in", v.shown)
    } else if !v.has_out {
        "Not sent: this port has no output".to_string()
    } else {
        "Not sent: its output didn't open (the log says why)".to_string()
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

/// "+1.0 ms", "0.0 ms", "-1.5 ms".
fn nudge_text(tenths: i16) -> String {
    if tenths == 0 {
        "0.0 ms".to_string()
    } else {
        format!("{:+.1} ms", f32::from(tenths) / 10.0)
    }
}

fn test_text(v: &View) -> String {
    format!("Test note C4 · ch {}", v.test_channel + 1)
}

/// What the delay card says under its value.
/// The clock card's line: why its clock waits, or what was measured as
/// it left.
fn clock_words(v: &View) -> String {
    if !v.out_on {
        return "Kept: MIDI out is off (Prefs).".to_string();
    }
    if !v.has_out {
        return "Kept: this port has no output.".to_string();
    }
    if !v.sending {
        return "Kept: its output didn't open.".to_string();
    }
    match v.measured {
        Some((gap, spread)) if v.setup.clock_out != ClockOut::Off => {
            format!("Pulses leave every {gap:.1} ms, ±{spread:.1} ms.")
        }
        // Clock is the Sync section's setting; Nudge is the gear's.
        _ => "Clock as on Sync; Nudge kept for every project.".to_string(),
    }
}

fn delay_words(v: &View) -> [String; 2] {
    let d = v.setup.delay_ms();
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
            &v.shown,
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
        let row = Rect::new(body.x, l.name.y, NAME_W, BOX_H);
        cx.text(FontId::Sans, t.size_body, row, Align::Left, t.text, "Name");
        value_box(cx, l.name, &v.shown, picked == Sel::Name, v.usb);
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
            PortInput::LearnOnly => "Learn hears it; only its routes play.",
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
        let row = Rect::new(body.x, l.profile.y, NAME_W, BOX_H);
        cx.text(
            FontId::Sans,
            t.size_body,
            row,
            Align::Left,
            t.text,
            "Profile",
        );
        value_box(
            cx,
            l.profile,
            &v.profile_label,
            picked == Sel::Profile,
            true,
        );
        lines(
            cx,
            Rect::new(body.x, l.profile.bottom() + 6, body.w, LINE_H),
            &[&v.profile_says],
            t.text_faint,
        );
        let calls = if v.shown != v.name {
            format!("{} · it calls itself {}", v.about, v.name)
        } else {
            v.about.clone()
        };
        lines(
            cx,
            Rect::new(body.x, l.profile.bottom() + 6 + LINE_H + 4, body.w, LINE_H),
            &[&calls],
            t.text_dim,
        );

        // Ignore coming in.
        let body = card(
            cx,
            l.ignore,
            "IGNORE COMING IN",
            if l.one_row {
                "lit = ignored · still in the Monitor"
            } else {
                "lit = ignored"
            },
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
            // A narrow chip takes the smaller letters, so "All notes off"
            // fits.
            let px = if cr.w < CHIP_W {
                t.size_small * 0.9
            } else {
                t.size_small
            };
            cx.text(FontId::Bold, px, cr, Align::Center, ink, kind.label());
        }
        if !l.one_row {
            lines(
                cx,
                Rect::new(body.x, l.chips[3].bottom() + 6, body.w, LINE_H),
                &["Ignored messages still show in the Monitor."],
                t.text_faint,
            );
        }

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
            &["Change where it goes in the Routing section."],
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
        let label = |cx: &mut PaintCx, r: Rect, s: &str| {
            cx.text(FontId::Sans, t.size_body, r, Align::Left, t.text, s);
        };
        let one_line = l.divide_label.y == l.nudge_label.y;
        label(
            cx,
            l.divide_label,
            if one_line { "Divide" } else { "Divide by" },
        );
        let (div, nudge) = (v.setup.clock_divide, v.setup.clock_nudge_tenths);
        button(cx, l.divide[0], "−", false, v.has_out && div > 1);
        value_box(
            cx,
            l.divide[1],
            &div.to_string(),
            picked == Sel::Divide,
            v.has_out,
        );
        button(
            cx,
            l.divide[2],
            "+",
            false,
            v.has_out && div < MAX_CLOCK_DIVIDE,
        );
        label(cx, l.nudge_label, "Nudge");
        button(
            cx,
            l.nudge[0],
            "−",
            false,
            v.has_out && nudge > -MAX_NUDGE_TENTHS,
        );
        value_box(
            cx,
            l.nudge[1],
            &nudge_text(nudge),
            picked == Sel::Nudge,
            v.has_out,
        );
        button(
            cx,
            l.nudge[2],
            "+",
            false,
            v.has_out && nudge < MAX_NUDGE_TENTHS,
        );
        lines(cx, l.clock_line, &[&clock_words(&v)], t.text_faint);

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
                "All notes off ends MPFree's notes on it,",
                "then sends CC 123 on all 16 channels.",
            ]
        } else if !v.out_on {
            [
                "Greyed while MIDI out is off (Prefs >",
                "Send MIDI and CV out).",
            ]
        } else if !v.plugged {
            ["Greyed: not plugged in.", ""]
        } else if !v.has_out {
            ["Greyed: this port has no output.", ""]
        } else {
            ["Greyed: its output didn't open (the log", "says why)."]
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
            Target::tap("port-name", 0, format!("Name: {}", v.shown), l.name).disabled_if(!v.usb),
            Target::tap(
                "port-input",
                0,
                format!("Input: {}", v.setup.input.label()),
                l.input,
            )
            .disabled_if(!v.has_in),
            Target::tap(
                "port-profile",
                0,
                format!("Profile: {}", v.profile_label),
                l.profile,
            ),
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
        let (div, nudge) = (v.setup.clock_divide, v.setup.clock_nudge_tenths);
        out.push(
            Target::tap("port-divide", 0, "Divide −", l.divide[0])
                .disabled_if(!v.has_out || div <= 1),
        );
        out.push(
            Target::tap("port-divide", 1, "Divide +", l.divide[2])
                .disabled_if(!v.has_out || div >= MAX_CLOCK_DIVIDE),
        );
        out.push(
            Target::tap("port-nudge", 0, "Nudge −", l.nudge[0])
                .disabled_if(!v.has_out || nudge <= -MAX_NUDGE_TENTHS),
        );
        out.push(
            Target::tap("port-nudge", 1, "Nudge +", l.nudge[2])
                .disabled_if(!v.has_out || nudge >= MAX_NUDGE_TENTHS),
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
                label: "Divide".to_string(),
                text: format!("÷ {}", v.setup.clock_divide),
                pos: f32::from(v.setup.clock_divide - 1) / f32::from(MAX_CLOCK_DIVIDE - 1),
                enabled: v.has_out,
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
            3 => self.step(Sel::Divide, d.signum(), false),
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

    /// The page as the I/O view gives it on the MPC's 1280 x 800 screen
    /// in the preview, and in the app, beside the Q-Link column.
    const WIDE: Rect = Rect::new(194, 136, 912, 614);
    const APP: Rect = Rect::new(194, 136, 772, 614);

    #[test]
    fn every_control_fits_and_says_what_it_is() {
        let m = model();
        for area in [AREA, WIDE, APP] {
            for name in [DIN_NAME, "Keystep 37"] {
                let p = PortPage::new(m.clone(), name);
                let _ = crate::form::testing::paint_in(&p, area);
                let ts = p.targets(area);
                assert_eq!(ts.len(), 17, "{name}");
                for t in ts {
                    let r = t.rect;
                    assert!(
                        r.x >= area.x
                            && r.y >= area.y
                            && r.right() <= area.right()
                            && r.y + r.h <= area.y + area.h,
                        "{name}: {} \"{}\" at {r:?} is outside {area:?}",
                        t.id,
                        t.label
                    );
                }
            }
        }
        // On the MPC's screen Divide and Nudge share a line, and the
        // Ignore chips sit in a row.
        for area in [WIDE, APP] {
            let l = layout(area);
            assert_eq!(l.divide[1].y, l.nudge[1].y);
            assert!(l.one_row);
            assert!(l.divide[2].right() < l.nudge_label.x);
        }
        // The line under Profile fits the app's page, with or without a
        // profile.
        let l = layout(APP);
        let w = l.input.right() - l.name.x + NAME_W;
        let mut fonts = mpfree_ui::Fonts::embedded();
        let px = mpfree_ui::Theme::dark().size_small;
        let gm = mpfree_midi::profile::Profiles::builtin();
        // A drum machine's: its controls, programs and 128 pads.
        let mut drums = mpfree_midi::profile::Profile::new("Drum box");
        for k in 0..128u8 {
            drums.set_cc(k, "Level");
            drums.set_program(k, "Kit");
            let out = mpfree_midi::profile::PadOut {
                note: k,
                channel: Some(9),
            };
            drums.set_pad(k, out);
        }
        drums.set_nrpn(0, 1, "Tune");
        assert_eq!(
            profile_says(Some(&drums), &ProfilePick::Auto),
            "Covers 128 CCs, 1 NRPN, 128 programs, 128 pads."
        );
        drums.set_pad(
            0,
            mpfree_midi::profile::PadOut {
                note: 36,
                channel: None,
            },
        );
        let mut pads_only = mpfree_midi::profile::Profile::new("Pads");
        pads_only.set_pad(
            0,
            mpfree_midi::profile::PadOut {
                note: 36,
                channel: None,
            },
        );
        assert_eq!(
            profile_says(Some(&pads_only), &ProfilePick::Auto),
            "Covers 1 kit pad."
        );
        // And the line under Divide and Nudge, whatever it says.
        let mut v = PortPage::new(m.clone(), DIN_NAME).view();
        v.out_on = true;
        v.has_out = true;
        v.sending = true;
        v.setup.clock_out = ClockOut::Always;
        let idle = clock_words(&v);
        v.measured = Some((20.8, 12.4));
        let measured = clock_words(&v);
        for says in [idle, measured] {
            let sw = fonts.measure(FontId::Sans, px, &says);
            assert!(
                sw <= l.clock_line.w,
                "{says:?} is {sw} wide, the line {}",
                l.clock_line.w
            );
        }
        for says in [
            profile_says(None, &ProfilePick::Auto),
            profile_says(None, &ProfilePick::Named("Juno 106 rack".into())),
            profile_says(gm.get("General MIDI"), &ProfilePick::Auto),
            profile_says(Some(&drums), &ProfilePick::Auto),
        ] {
            let sw = fonts.measure(FontId::Sans, px, &says);
            assert!(sw <= w, "{says:?} is {sw} wide, the line {w}");
        }
        let l = layout(AREA);
        assert!(l.nudge[1].y > l.divide[1].y && !l.one_row);
    }

    /// The clock card: Divide (the song's) slows the port's clock, Nudge
    /// (the gear's) moves it later or earlier, each stopping at its ends;
    /// the line under them says what was measured as the clock left.
    #[test]
    fn the_clock_card_divides_nudges_and_says_what_it_measured() {
        let m = model();
        m.lock().unwrap().live.built.notes_out = true;
        {
            let mut g = m.lock().unwrap();
            let p = g
                .live
                .ports
                .iter_mut()
                .find(|p| p.name == DIN_NAME)
                .unwrap();
            p.sending = true;
        }
        let mut p = PortPage::new(m.clone(), DIN_NAME);
        let l = layout(AREA);
        for _ in 0..3 {
            let (x, y) = centre(l.divide[2]);
            tap(&mut p, x, y);
        }
        assert_eq!(port(&m, DIN_NAME).clock_divide, 4);
        let (x, y) = centre(l.divide[0]);
        tap(&mut p, x, y);
        assert_eq!(port(&m, DIN_NAME).clock_divide, 3);
        // Q-Link 4 turns it, and stops at 8.
        for _ in 0..9 {
            p.qlink_turn(3, 1, false);
        }
        assert_eq!(port(&m, DIN_NAME).clock_divide, MAX_CLOCK_DIVIDE);
        assert!(p.take_note().unwrap().contains("at its most"));
        // Nudge: two steps earlier.
        for _ in 0..2 {
            let (x, y) = centre(l.nudge[0]);
            tap(&mut p, x, y);
        }
        assert_eq!(
            port(&m, DIN_NAME).clock_nudge_tenths,
            -2 * NUDGE_STEP_TENTHS
        );
        assert_eq!(nudge_text(-10), "-1.0 ms");
        assert_eq!(nudge_text(5), "+0.5 ms");
        // The wheel goes Clock, Divide, Nudge; - and + step the nudge to
        // its latest.
        p.sel = SELS.iter().position(|s| *s == Sel::Nudge).unwrap();
        for _ in 0..100 {
            p.input(IoInput::Step(1));
        }
        assert_eq!(port(&m, DIN_NAME).clock_nudge_tenths, MAX_NUDGE_TENTHS);
        assert!(p.take_note().unwrap().contains("at its latest"));
        // The song keeps the divide; the rig, the nudge.
        let g = m.lock().unwrap();
        assert_eq!(g.setup.project().port(DIN_NAME).clock_divide, 8);
        assert_eq!(
            g.setup.rig().port(DIN_NAME).clock_nudge_tenths,
            MAX_NUDGE_TENTHS
        );
        drop(g);
        // What was measured, while clock goes out.
        let words = |m: &Shared| clock_words(&view(&m.lock().unwrap(), DIN_NAME, 0.0));
        m.lock()
            .unwrap()
            .setup
            .set_port(DIN_NAME, |p| p.clock_out = ClockOut::Always);
        assert_eq!(words(&m), "Clock as on Sync; Nudge kept for every project.");
        {
            let mut g = m.lock().unwrap();
            let p = g
                .live
                .ports
                .iter_mut()
                .find(|p| p.name == DIN_NAME)
                .unwrap();
            p.clock_measured = Some((20.83, 0.31));
        }
        assert_eq!(words(&m), "Pulses leave every 20.8 ms, ±0.3 ms.");
        m.lock().unwrap().live.built.notes_out = false;
        assert_eq!(words(&m), "Kept: MIDI out is off (Prefs).");
    }

    /// Profile: Auto takes the profile made for the name the port calls
    /// itself; a tap steps to None, then each profile by name, and round;
    /// minus stops at Auto. The rig keeps it, and a profile that has gone
    /// says so.
    #[test]
    fn the_profile_names_the_ports_controls() {
        use mpfree_midi::profile::Profile;
        let m = model();
        let text = "mpfree-profile 1\nname = KeyStep\nmatch = Keystep 37\ncc.74 = Knob 1\n";
        let (keystep, _) = Profile::parse(text).unwrap();
        m.lock().unwrap().profiles.add(keystep).unwrap();
        let mut p = PortPage::new(m.clone(), "Keystep 37");
        let v = p.view();
        assert_eq!(v.profile_label, "Auto · KeyStep");
        assert_eq!(v.profile_says, "Names 1 control.");
        let used = |m: &Shared| {
            m.lock()
                .unwrap()
                .profile("Keystep 37")
                .map(|p| p.name.clone())
        };
        assert_eq!(used(&m).as_deref(), Some("KeyStep"));
        let l = layout(AREA);
        let (x, y) = centre(l.profile);
        tap(&mut p, x, y);
        assert_eq!(port(&m, "Keystep 37").profile, ProfilePick::Off);
        assert_eq!(used(&m), None);
        assert!(p.view().profile_says.contains("Usual names for controls"));
        tap(&mut p, x, y);
        let gm = ProfilePick::Named("General MIDI".into());
        assert_eq!(port(&m, "Keystep 37").profile, gm);
        assert_eq!(p.view().profile_says, "Names 17 controls and 128 programs.");
        tap(&mut p, x, y);
        tap(&mut p, x, y);
        assert_eq!(port(&m, "Keystep 37").profile, ProfilePick::Auto, "round");
        assert_eq!(SELS[p.sel], Sel::Profile);
        assert_eq!(p.input(IoInput::Step(-1)), IoReply::Handled);
        assert!(p.take_note().unwrap().contains("Profile is at its first"));
        // Gone since it was picked; the rig keeps the pick.
        let gone = ProfilePick::Named("Gone".into());
        m.lock()
            .unwrap()
            .setup
            .set_port("Keystep 37", |p| p.profile = gone.clone());
        assert_eq!(p.view().profile_label, "Gone (not found)");
        assert!(p.view().profile_says.starts_with("Not found"));
        assert_eq!(
            m.lock().unwrap().setup.rig().port("Keystep 37").profile,
            gone
        );
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
        p.input(IoInput::Wheel(7));
        assert_eq!(SELS[p.sel], Sel::Clock);
        assert_eq!(SELS[p.sel + 1], Sel::Divide);
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

    /// A USB device's Name asks the app for the text keyboard (the app
    /// sets the name); the 5-pin jacks keep the panel's names. The name
    /// given shows on the page, which still says the one it calls itself.
    #[test]
    fn a_usb_ports_name_asks_the_app_and_the_jacks_keep_theirs() {
        let m = model();
        let l = layout(AREA);
        let (x, y) = centre(l.name);
        let mut din = PortPage::new(m.clone(), DIN_NAME);
        tap(&mut din, x, y);
        assert_eq!(m.lock().unwrap().asks.name_port, None);
        assert!(din
            .take_note()
            .unwrap()
            .contains("keep the names on the MPC's panel"));
        let mut p = PortPage::new(m.clone(), "Keystep 37");
        assert_eq!(tap(&mut p, x, y), IoReply::Redraw);
        let asked = m.lock().unwrap().asks.name_port.take();
        assert_eq!(asked.as_deref(), Some("Keystep 37"));
        // Name is the wheel's first: - says how, + asks.
        p.sel = 0;
        assert_eq!(SELS[p.sel], Sel::Name);
        p.input(IoInput::Step(-1));
        assert!(p.take_note().unwrap().contains("types its name"));
        assert_eq!(m.lock().unwrap().asks.name_port, None);
        p.input(IoInput::Step(1));
        assert!(m.lock().unwrap().asks.name_port.is_some());
        // Named Keys by the app: the page and its Name say so.
        {
            let mut g = m.lock().unwrap();
            g.setup.set_shown("Keystep 37", "Keys", []).unwrap();
            g.gen += 1;
        }
        let v = view(&m.lock().unwrap(), "Keystep 37", 0.0);
        assert_eq!((v.name.as_str(), v.shown.as_str()), ("Keystep 37", "Keys"));
        let _ = paint(&p);
        let ts = p.targets(AREA);
        let name = ts.iter().find(|t| t.id == "port-name").unwrap();
        assert_eq!(name.label, "Name: Keys");
        assert!(name.enabled);
        let ts = din.targets(AREA);
        assert!(!ts.iter().find(|t| t.id == "port-name").unwrap().enabled);
    }

    /// MIDI out on, sending to every port with an output.
    fn sending(m: &Shared) {
        let mut g = m.lock().unwrap();
        g.live.built.notes_out = true;
        for p in g.live.ports.iter_mut() {
            p.sending = p.has_out;
        }
    }

    /// The sends ask the app only while MIDI out sends to the port, and
    /// say the app's answer; greyed, they say why.
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
        // On, but the jacks' output didn't open.
        m.lock().unwrap().live.built.notes_out = true;
        tap(&mut p, x, y);
        assert_eq!(m.lock().unwrap().asks.port_send, None);
        assert!(p.take_note().unwrap().contains("didn't open"));
        sending(&m);
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
        // A USB device MIDI out sends to asks the same way.
        let mut usb = PortPage::new(m.clone(), "Keystep 37");
        m.lock().unwrap().asks.port_send = None;
        let (x, y) = centre(l.test);
        tap(&mut usb, x, y);
        let ask = m.lock().unwrap().asks.port_send.clone().unwrap();
        assert_eq!(ask.port, "Keystep 37");
        // One that isn't plugged in says so.
        let mut gone = PortPage::new(m.clone(), "Digitakt");
        m.lock().unwrap().asks.port_send = None;
        tap(&mut gone, x, y);
        assert_eq!(m.lock().unwrap().asks.port_send, None);
        assert_eq!(
            gone.take_note().as_deref(),
            Some("Not sent: Digitakt is not plugged in")
        );
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
        // Its routes come first; the rest goes where its Input says.
        m.lock().unwrap().setup.routes = vec![mpfree_midi::setup::TrackRoute {
            port: "Keystep 37".into(),
            channel: Some(1),
            track: 1,
            blocks: mpfree_midi::blocks::Blocks::NONE,
        }];
        let v = view(&m.lock().unwrap(), "Keystep 37", 0.0);
        let words: Vec<&str> = v.does.iter().map(|d| d.1.as_str()).collect();
        assert_eq!(
            words[..2],
            ["Ch 2 → Bass", "The rest play nothing (Learn only)"],
            "{words:?}"
        );
        // Thru is listed with the routes, and on the output's page.
        m.lock().unwrap().setup.thru = vec![mpfree_midi::setup::ThruRoute {
            from: "Keystep 37".into(),
            channel: Some(9),
            to: DIN_NAME.into(),
            blocks: mpfree_midi::blocks::Blocks::NONE,
        }];
        let v = view(&m.lock().unwrap(), "Keystep 37", 0.0);
        assert_eq!(
            v.does[0].1, "Ch 2 → Bass, Ch 10 → MIDI out, thru",
            "{:?}",
            v.does
        );
        let v = view(&m.lock().unwrap(), DIN_NAME, 0.0);
        assert!(
            v.does
                .iter()
                .any(|d| !d.0 && d.1 == "Thru from Keystep 37 · ch 10"),
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
