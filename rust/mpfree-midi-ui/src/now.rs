//! The Monitor's Now view: what a port's channel holds now, rather than
//! the list of what came (board 2 of the MIDI mockups, 16 Nov 2026).
//!
//! - **Keys down** on a keyboard drawing, with each note's velocity and
//!   pressure, the sustain pedal, how long the newest has been held and
//!   where it went.
//! - **Controls that moved,** newest first: each controller's last value
//!   as a bar, named when the controller is a common one, and the
//!   parameter it moves when it is learned. The last program and bank.
//! - **Clock:** the tempo of the clock on the port picked (or the first
//!   one arriving), the spread of its pulse gaps, the worst gap of the last
//!   minute, pulses lost or late, and a bar for each second's worst gap.
//! - **Bend and pressure** on the channel.
//! - **Notes still on** going out of each port, the longest held first:
//!   one held 30 s or more is called stuck. "Send note-offs to MIDI out"
//!   ends the notes MPFree itself left on (only those it sent, never a
//!   blind All notes off); it is greyed while MIDI out is off.
//!
//! Everything comes from [`mpfree_midi::monitor::Monitor`]: what is held
//! from the lines it lists ([`Held`]), the clock from its feed
//! ([`Pulses`]). Hold freezes the view as it is ([`NowData`] is a copy).

use mpfree_midi::learn::Source;
use mpfree_midi::monitor::now::{Held, Pulses, LATE, NEVER, NO_TRACK, STUCK_S};
use mpfree_midi::monitor::{Dir, Monitor, To};
use mpfree_midi::port::PortKind;
use mpfree_midi::setup::TempoSource;
use mpfree_midi::PortId;
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect};

use crate::monitor::{port_text, went_ink, went_to};
use crate::{note_name, IoModel};
use mpfree_midi::profile::cc_name;

/// The keyboard's keys: white keys drawn, from C.
const WHITE: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
const OCTAVES: u8 = 3;
/// The lowest key drawn while every note held fits (C3).
const BASE: u8 = 48;
/// The highest the keyboard starts (C7): its last keys past G9 are left
/// out.
const TOP_BASE: u8 = 96;

const CARD_GAP: i32 = 10;
const ROW_H: i32 = 34;

/// The clock shown: its port and way, its tempo and pulses, and whether
/// the tempo follows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Clock {
    pub name: String,
    pub bpm: Option<f32>,
    pub pulses: Pulses,
    pub followed: bool,
    pub dir: Dir,
}

/// A note still on going out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StillOn {
    /// "MIDI out · ch 2 · C2".
    pub what: String,
    pub held_s: f32,
    /// "sent by Bass", if a track sent it.
    pub by: String,
}

/// Everything the view shows, copied out of the model (Hold keeps one).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NowData {
    pub stream: (PortId, Dir),
    /// The port's name ("Keystep 37").
    pub name: String,
    /// 0 to 15.
    pub channel: u8,
    pub held: Held,
    /// Where the newest note went, in words.
    pub went: String,
    /// Each learned controller on this port and channel, with what it
    /// moves in words.
    pub learned: Vec<(u8, String)>,
    /// The names of the controllers that moved: the port's profile's,
    /// else the ones most gear agrees on.
    pub names: Vec<(u8, String)>,
    /// The last program's name in the port's profile.
    pub program_name: Option<String>,
    pub clock: Option<Clock>,
    pub still: Vec<StillOn>,
    pub now: f64,
    /// MIDI Out is on (Send note-offs can do something).
    pub out_on: bool,
}

/// The ports and ways the view can show: each port's input, and MIDI
/// Out's (the only output MPFree writes).
pub(crate) fn streams(m: &Monitor) -> Vec<(PortId, Dir)> {
    let mut v = Vec::new();
    for p in m.shown_ports() {
        if p.has_in {
            v.push((p.id, Dir::In));
        }
        if p.kind == PortKind::Din {
            v.push((p.id, Dir::Out));
        }
    }
    v
}

/// When a port's channel last had a message going `dir`.
fn last(m: &Monitor, (p, d): (PortId, Dir)) -> Option<(f64, u8)> {
    (0..16u8)
        .filter_map(|c| m.last(p, d, c).map(|t| (t, c)))
        .max_by(|a, b| a.0.total_cmp(&b.0))
}

/// The port and way to show when none is picked: the one with the newest
/// channel message, inputs first; else the 5-pin jacks' input.
pub(crate) fn auto_stream(m: &Monitor) -> (PortId, Dir) {
    let all = streams(m);
    let newest = |d: Dir| {
        all.iter()
            .filter(|s| s.1 == d)
            .filter_map(|&s| last(m, s).map(|(t, _)| (t, s)))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, s)| s)
    };
    newest(Dir::In)
        .or_else(|| newest(Dir::Out))
        .unwrap_or((mpfree_midi::DIN, Dir::In))
}

/// The channel to show when none is picked: the one with the newest
/// message, else channel 1.
pub(crate) fn auto_channel(m: &Monitor, s: (PortId, Dir)) -> u8 {
    last(m, s).map_or(0, |(_, c)| c)
}

/// The view's data for `stream`'s `channel` at `now`.
pub(crate) fn data(m: &IoModel, stream: (PortId, Dir), channel: u8, now: f64) -> NowData {
    let mon = &m.monitor;
    let name = mon.port_name(stream.0).unwrap_or("Port").to_string();
    // The settings name a port by its own name, not one given to it.
    let own = mon.own_name(stream.0).unwrap_or("Port").to_string();
    let held = mon
        .held(stream.0, stream.1, channel)
        .cloned()
        .unwrap_or_default();
    let went = went_to(m, held.note_to).0;
    let profile = m.profile(&own);
    let names = held
        .moved()
        .iter()
        .filter_map(|&(cc, _, _)| Some((cc, cc_name(profile, cc)?.to_string())))
        .collect();
    let program_name = profile
        .and_then(|p| p.program_name(held.program))
        .map(str::to_string);
    let learned = m
        .setup
        .learn
        .iter()
        .enumerate()
        .filter(|(_, l)| l.channel == channel && l.port.as_ref().is_none_or(|p| *p == own))
        .filter_map(|(i, l)| match l.source {
            Source::Cc(cc) if stream.1 == Dir::In => Some((
                cc,
                m.live
                    .learn_names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| l.target.to_string()),
            )),
            _ => None,
        })
        .collect();
    let clock_of = |s: (PortId, Dir)| {
        let p = mon.pulses(s.0, s.1)?;
        let name = mon.port_name(s.0).unwrap_or("Port").to_string();
        let own = mon.own_name(s.0).unwrap_or("Port");
        let followed = s.1 == Dir::In && matches!(&m.setup.tempo, TempoSource::Port(n) if n == own);
        Some(Clock {
            name,
            bpm: mon.bpm(s.0, s.1),
            pulses: *p,
            followed,
            dir: s.1,
        })
    };
    let clock = clock_of(stream).or_else(|| streams(mon).into_iter().find_map(clock_of));
    let still = mon
        .still_on()
        .into_iter()
        .map(|(p, c, n, h)| StillOn {
            what: format!(
                "{} · ch {} · {}",
                port_text(mon, p, Dir::Out),
                c + 1,
                note_name(n)
            ),
            held_s: (now as f32 - h.down_at[usize::from(n)]).max(0.0),
            by: match h.by[usize::from(n)] {
                NO_TRACK => String::new(),
                t => went_to(m, To::SentBy(t)).0,
            },
        })
        .collect();
    NowData {
        stream,
        name,
        channel,
        held,
        went,
        learned,
        names,
        program_name,
        clock,
        still,
        now,
        out_on: m.live.built.notes_out,
    }
}

/// A number that changes when what the view shows changes (it shows
/// times to the second).
pub(crate) fn live(d: &NowData) -> u64 {
    let h = &d.held;
    let mut x = d.now as u64;
    for (k, v) in h
        .vel
        .iter()
        .chain(h.pressure.iter())
        .chain(h.cc.iter())
        .enumerate()
    {
        x = x.rotate_left(5) ^ (u64::from(*v) << (k % 7));
    }
    x ^= u64::from(h.bend as u16) << 20
        ^ u64::from(h.chan_pressure) << 40
        ^ u64::from(h.program) << 48;
    x ^= h.cc_at.iter().filter(|&&t| t > NEVER).count() as u64;
    if let Some(c) = &d.clock {
        x ^= ((c.pulses.gap_ms * 10.0) as u64) << 8
            ^ u64::from(c.pulses.late) << 24
            ^ (c.pulses.seconds().count() as u64) << 36
            ^ c.bpm.map_or(0, |b| (b * 10.0) as u64) << 44;
    }
    x ^ (d.still.len() as u64) << 56 ^ u64::from(d.out_on) << 63
}

/// Where each card goes in the view's area.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cards {
    pub keys: Rect,
    pub controls: Rect,
    pub clock: Rect,
    pub bend: Rect,
    pub still: Rect,
    pub note_offs: Rect,
}

pub(crate) fn cards(area: Rect) -> Cards {
    let left_w = (area.w - CARD_GAP) * 62 / 100;
    let (left, rest) = area.take_left(left_w);
    let right = Rect::new(rest.x + CARD_GAP, rest.y, rest.w - CARD_GAP, rest.h);
    let keys_h = 196.min(left.h / 2);
    let (keys, controls) = left.take_top(keys_h);
    let controls = Rect::new(
        controls.x,
        controls.y + CARD_GAP,
        controls.w,
        controls.h - CARD_GAP,
    );
    let clock_h = 222.min(right.h * 2 / 5);
    let (clock, rest) = right.take_top(clock_h);
    let bend = Rect::new(rest.x, rest.y + CARD_GAP, rest.w, 118);
    let still = Rect::new(
        rest.x,
        bend.bottom() + CARD_GAP,
        rest.w,
        (rest.bottom() - bend.bottom() - CARD_GAP).max(0),
    );
    let note_offs = Rect::new(
        still.x + 12,
        still.bottom() - 12 - 22 - 44,
        still.w - 24,
        44,
    );
    Cards {
        keys,
        controls,
        clock,
        bend,
        still,
        note_offs,
    }
}

/// Seconds as "just now", "6 s ago", "2 min ago".
pub(crate) fn ago(s: f64) -> String {
    if s < 1.0 {
        "just now".to_string()
    } else if s < 60.0 {
        format!("{} s ago", s as u32)
    } else {
        format!("{} min ago", (s / 60.0) as u32)
    }
}

/// Seconds held: "1 s", "41 s", "2 min".
fn held_for(s: f32) -> String {
    if s < 60.0 {
        format!("{} s", s as u32)
    } else {
        format!("{} min", (s / 60.0) as u32)
    }
}

/// A card: its panel and title, and the area below the title.
pub(crate) fn card(cx: &mut PaintCx, r: Rect, title: &str, right: &str, right_ink: Color) -> Rect {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(r, t.round(10), t.surface);
    let head = Rect::new(r.x + 14, r.y + 8, r.w - 28, 24);
    cx.text(
        FontId::Bold,
        t.size_small,
        head,
        Align::Left,
        t.text_dim,
        title,
    );
    if !right.is_empty() {
        cx.text(
            FontId::Bold,
            t.size_small,
            head,
            Align::Right,
            right_ink,
            right,
        );
    }
    Rect::new(r.x + 14, r.y + 38, r.w - 28, (r.h - 46).max(0))
}

/// A row's label on the left and its value on the right.
fn kv(cx: &mut PaintCx, r: Rect, k: &str, v: &str, v_ink: Color) {
    let t = cx.theme.clone();
    cx.text(FontId::Sans, t.size_small, r, Align::Left, t.text_dim, k);
    cx.text(FontId::Mono, t.size_small, r, Align::Right, v_ink, v);
}

/// The lowest key drawn, always a C: C3, or the C below the lowest note
/// held when one is out of that range.
fn keyboard_base(h: &Held) -> u8 {
    let (lo, hi) = (h.down().next(), h.down().last());
    let span = 12 * OCTAVES;
    match (lo, hi) {
        (Some(lo), Some(hi)) if lo < BASE || hi >= BASE + span => (lo / 12 * 12).min(TOP_BASE),
        _ => BASE,
    }
}

fn paint_keys(cx: &mut PaintCx, d: &NowData, r: Rect) {
    let t = cx.theme.clone();
    let h = &d.held;
    let n = h.count();
    let right = match h.sustain() {
        Some(true) => format!("{n} held · sustain down"),
        Some(false) => format!("{n} held · sustain up"),
        None => format!("{n} held"),
    };
    let title = format!(
        "KEYS DOWN · {} · CH {}",
        d.name.to_uppercase(),
        d.channel + 1
    );
    let body = card(
        cx,
        r,
        &title,
        &right,
        if n > 0 { t.good } else { t.text_faint },
    );
    let kb = Rect::new(body.x, body.y, body.w, 84.min(body.h));
    let base = keyboard_base(h);
    let whites: Vec<u8> = (0..OCTAVES)
        .flat_map(|o| WHITE.iter().map(move |w| base + o * 12 + w))
        .collect();
    let ww = kb.w / whites.len() as i32;
    let piano = t.keys();
    let white = if t.is_light() { piano.white } else { t.text };
    for (i, &note) in whites.iter().enumerate() {
        if note > 127 {
            continue;
        }
        let key = Rect::new(kb.x + i as i32 * ww, kb.y, ww - 2, kb.h);
        let vel = h.vel[usize::from(note)];
        cx.canvas
            .fill_round_rect(key, 3, if vel > 0 { t.good } else { white });
        let foot = Rect::new(key.x, key.bottom() - 20, key.w, 18);
        if vel > 0 {
            cx.text(
                FontId::Bold,
                t.size_small * 0.8,
                foot,
                Align::Center,
                t.ink_on(t.good),
                &vel.to_string(),
            );
        } else if note % 12 == 0 {
            cx.text(
                FontId::Sans,
                t.size_small * 0.75,
                foot.inset_xy(2, 0),
                Align::Left,
                t.readable(t.text_faint, white),
                &note_name(note),
            );
        }
    }
    let bw = ww * 3 / 5;
    for (i, &note) in whites.iter().enumerate() {
        // A black key right of C, D, F, G and A.
        if !matches!(note % 12, 0 | 2 | 5 | 7 | 9) || note >= 127 {
            continue;
        }
        let x = kb.x + (i as i32 + 1) * ww - bw / 2 - 1;
        let key = Rect::new(x, kb.y, bw, kb.h * 3 / 5);
        let held = h.vel[usize::from(note + 1)] > 0;
        cx.canvas
            .fill_round_rect(key, 2, if held { t.good } else { piano.black });
    }
    let line = Rect::new(body.x, kb.bottom() + 8, body.w, 24);
    let notes: Vec<String> = h
        .down()
        .map(|n| {
            let i = usize::from(n);
            let mut s = format!("{} · vel {}", note_name(n), h.vel[i]);
            if h.pressure[i] > 0 {
                s.push_str(&format!(" · pressure {}", h.pressure[i]));
            }
            s
        })
        .collect();
    if notes.is_empty() {
        cx.text(
            FontId::Sans,
            t.size_small,
            line,
            Align::Left,
            t.text_faint,
            "No keys down on this channel",
        );
    } else {
        cx.text(
            FontId::Sans,
            t.size_small,
            line,
            Align::Left,
            t.text,
            &notes.join("    "),
        );
    }
    let newest = h.down().map(|n| h.down_at[usize::from(n)]).reduce(f32::max);
    let mut about = Vec::new();
    if let Some(at) = newest {
        about.push(format!("held {}", held_for((d.now as f32 - at).max(0.0))));
    }
    if !d.went.is_empty() {
        about.push(d.went.clone());
    }
    if !about.is_empty() {
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(line.x, line.bottom() + 6, line.w, 22),
            Align::Left,
            t.text_dim,
            &about.join(" · "),
        );
    }
}

fn paint_controls(cx: &mut PaintCx, d: &NowData, r: Rect) {
    let t = cx.theme.clone();
    let h = &d.held;
    let mut right = Vec::new();
    if h.program_at > NEVER {
        right.push(match &d.program_name {
            Some(n) => format!("PROGRAM {} {n}", u16::from(h.program) + 1),
            None => format!("PROGRAM {}", u16::from(h.program) + 1),
        });
    }
    if let Some((msb, lsb)) = h.bank() {
        right.push(format!("BANK {msb}/{lsb}"));
    }
    let body = card(
        cx,
        r,
        "CONTROLS THAT MOVED · NEWEST FIRST",
        &right.join(" · "),
        t.text_dim,
    );
    let moved = h.moved();
    if moved.is_empty() {
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(body.x, body.y, body.w, 28),
            Align::Left,
            t.text_faint,
            "No controller has moved on this channel yet",
        );
        return;
    }
    let fit = ((body.h - 24) / ROW_H).max(1) as usize;
    let name_w = 160.min(body.w / 3);
    let bar_w = 110.min(body.w / 5);
    for (k, &(cc, value, at)) in moved.iter().take(fit).enumerate() {
        let row = Rect::new(body.x, body.y + k as i32 * ROW_H, body.w, ROW_H - 4);
        let name = match d.names.iter().find(|n| n.0 == cc) {
            Some((_, n)) => format!("CC {cc} {n}"),
            None => format!("CC {cc}"),
        };
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(row.x, row.y, name_w - 8, row.h),
            Align::Left,
            t.text,
            &name,
        );
        let learned = d.learned.iter().find(|l| l.0 == cc);
        let track = Rect::new(row.x + name_w, row.y + row.h / 2 - 4, bar_w, 8);
        cx.canvas.fill_round_rect(track, 4, t.raised);
        let fill = track.w * i32::from(value) / 127;
        if fill > 0 {
            cx.canvas.fill_round_rect(
                Rect::new(track.x, track.y, fill, track.h),
                4,
                if learned.is_some() {
                    t.accent2
                } else {
                    t.accent
                },
            );
        }
        cx.text(
            FontId::Mono,
            t.size_small,
            Rect::new(track.right() + 10, row.y, 40, row.h),
            Align::Left,
            t.text,
            &value.to_string(),
        );
        let x = track.right() + 56;
        let rest = Rect::new(x, row.y, (row.right() - x).max(0), row.h);
        let since = ago(d.now - f64::from(at));
        let (text, ink) = match (learned, cc) {
            (Some((_, what)), _) => (format!("Learn: {what}"), t.accent2),
            (None, 64) => (
                format!("{} · {since}", if value >= 64 { "down" } else { "up" }),
                t.text_dim,
            ),
            (None, _) => (since, t.text_dim),
        };
        cx.text(FontId::Sans, t.size_small, rest, Align::Left, ink, &text);
    }
    cx.text(
        FontId::Sans,
        t.size_small * 0.9,
        Rect::new(body.x, body.bottom() - 20, body.w, 20),
        Align::Left,
        t.text_faint,
        "Names come from the common CC list.",
    );
}

fn paint_clock(cx: &mut PaintCx, d: &NowData, r: Rect) {
    let t = cx.theme.clone();
    let Some(c) = &d.clock else {
        let body = card(cx, r, "CLOCK", "", t.text_faint);
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(body.x, body.y, body.w, 28),
            Align::Left,
            t.text_faint,
            "No clock is arriving or going out",
        );
        return;
    };
    let way = if c.dir == Dir::In { "IN" } else { "OUT" };
    let (state, ink) = match (c.followed, c.dir) {
        (true, _) => ("FOLLOWED", t.good),
        (false, Dir::In) => ("ARRIVING", t.text_dim),
        (false, Dir::Out) => ("GOING OUT", t.text_dim),
    };
    let body = card(
        cx,
        r,
        &format!("CLOCK · {} {way}", c.name.to_uppercase()),
        state,
        ink,
    );
    let p = &c.pulses;
    let rows: [(&str, String, Color); 4] = [
        (
            "Tempo",
            c.bpm
                .map_or("measuring".to_string(), |b| format!("{b:.1} BPM")),
            t.text,
        ),
        (
            "Pulse gap, spread",
            if p.gap_ms > 0.0 {
                format!("{:.1} · ±{:.1} ms", p.gap_ms, p.spread_ms)
            } else {
                "measuring".to_string()
            },
            t.text,
        ),
        (
            "Worst gap, last minute",
            p.worst()
                .map_or("measuring".to_string(), |w| format!("{w:.1} ms")),
            t.text,
        ),
        (
            "Lost or late pulses",
            p.late.to_string(),
            if p.late == 0 { t.good } else { t.warn },
        ),
    ];
    for (k, (key, value, ink)) in rows.iter().enumerate() {
        kv(
            cx,
            Rect::new(body.x, body.y + k as i32 * 26, body.w, 24),
            key,
            value,
            *ink,
        );
    }
    let bars = Rect::new(body.x, body.y + 4 * 26 + 6, body.w, 30);
    let worst: Vec<f32> = p.seconds().collect();
    let shown = &worst[worst.len().saturating_sub(16)..];
    let bw = bars.w / 16;
    for i in 0..16 {
        let x = bars.x + i * bw;
        let slot = Rect::new(x, bars.y, bw - 3, bars.h);
        cx.canvas.fill_rect(slot, t.raised);
        let k = i as usize + shown.len();
        if k < 16 {
            continue;
        }
        let w = shown[k - 16];
        let scale = if p.gap_ms > 0.0 {
            (w / (2.0 * p.gap_ms)).clamp(0.1, 1.0)
        } else {
            0.1
        };
        let bh = (bars.h as f32 * scale) as i32;
        let late = p.gap_ms > 0.0 && w > LATE * p.gap_ms;
        cx.canvas.fill_rect(
            Rect::new(x, bars.bottom() - bh, bw - 3, bh),
            if late { t.warn } else { t.text_faint },
        );
    }
    cx.text(
        FontId::Sans,
        t.size_small * 0.85,
        Rect::new(body.x, bars.bottom() + 6, body.w, 20),
        Align::Left,
        t.text_faint,
        "Bars: each second's worst gap.",
    );
}

fn paint_bend(cx: &mut PaintCx, d: &NowData, r: Rect) {
    let t = cx.theme.clone();
    let h = &d.held;
    let body = card(
        cx,
        r,
        &format!("BEND AND PRESSURE · CH {}", d.channel + 1),
        "",
        t.text_dim,
    );
    let bend = if h.bend > 0 {
        format!("+{}", h.bend)
    } else {
        h.bend.to_string()
    };
    kv(
        cx,
        Rect::new(body.x, body.y, body.w, 22),
        "Pitch bend",
        &bend,
        t.text,
    );
    let track = Rect::new(body.x, body.y + 26, body.w, 8);
    cx.canvas.fill_round_rect(track, 4, t.raised);
    let mid = track.x + track.w / 2;
    let span = track.w / 2 * i32::from(h.bend) / 8192;
    let (x0, x1) = if span >= 0 {
        (mid - 4, mid + span + 4)
    } else {
        (mid + span - 4, mid + 4)
    };
    cx.canvas
        .fill_round_rect(Rect::new(x0, track.y, x1 - x0, 8), 4, t.accent);
    kv(
        cx,
        Rect::new(body.x, body.y + 40, body.w, 22),
        "Channel pressure",
        &h.chan_pressure.to_string(),
        t.text,
    );
    let track = Rect::new(body.x, body.y + 66, body.w, 8);
    cx.canvas.fill_round_rect(track, 4, t.raised);
    let fill = track.w * i32::from(h.chan_pressure) / 127;
    if fill > 0 {
        cx.canvas
            .fill_round_rect(Rect::new(track.x, track.y, fill, 8), 4, t.accent);
    }
}

fn paint_still(cx: &mut PaintCx, d: &NowData, r: Rect, button: Rect) {
    let t = cx.theme.clone();
    let stuck = d.still.iter().filter(|s| s.held_s >= STUCK_S).count();
    let (right, ink) = if stuck > 0 {
        (format!("{stuck} STUCK"), t.warn)
    } else if d.still.is_empty() {
        ("NONE".to_string(), t.text_faint)
    } else {
        (format!("{} ON", d.still.len()), t.text_dim)
    };
    let body = card(cx, r, "NOTES STILL ON · MIDI OUT", &right, ink);
    let room = ((button.y - body.y) / 44).max(0) as usize;
    if d.still.is_empty() {
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(body.x, body.y, body.w, 24),
            Align::Left,
            t.text_faint,
            "Every note MPFree sent has ended.",
        );
    }
    for (k, s) in d.still.iter().take(room).enumerate() {
        let y = body.y + k as i32 * 44;
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(body.x, y, body.w, 20),
            Align::Left,
            t.text,
            &s.what,
        );
        let mut about = format!("on for {}", held_for(s.held_s));
        if !s.by.is_empty() {
            about.push_str(" · ");
            about.push_str(&s.by);
        }
        let stuck = s.held_s >= STUCK_S;
        cx.text(
            FontId::Sans,
            t.size_small * 0.9,
            Rect::new(body.x, y + 20, body.w, 20),
            Align::Left,
            if stuck {
                t.warn
            } else {
                went_ink(&t, crate::monitor::Went::Sent)
            },
            &about,
        );
    }
    let on = can_end(d);
    let (fill, text) = if on {
        (t.raised, t.text)
    } else {
        (t.bg, t.text_faint)
    };
    cx.canvas.fill_round_rect(button, t.round(10), fill);
    cx.text(
        FontId::Bold,
        t.size_small,
        button,
        Align::Center,
        text,
        "Send note-offs to MIDI out",
    );
    cx.text(
        FontId::Sans,
        t.size_small * 0.85,
        Rect::new(body.x, button.bottom() + 2, body.w, 20),
        Align::Left,
        t.text_faint,
        "Ends only the notes MPFree left on.",
    );
}

/// Whether "Send note-offs to MIDI out" can do something: MIDI Out is on
/// and a note is still on there.
pub(crate) fn can_end(d: &NowData) -> bool {
    d.out_on && !d.still.is_empty()
}

pub(crate) fn paint(cx: &mut PaintCx, d: &NowData, c: &Cards) {
    paint_keys(cx, d, c.keys);
    paint_controls(cx, d, c.controls);
    paint_clock(cx, d, c.clock);
    paint_bend(cx, d, c.bend);
    paint_still(cx, d, c.still, c.note_offs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;

    #[test]
    fn the_view_follows_the_newest_port_and_channel_and_says_what_is_held() {
        let m = sample::monitored(sample::rig());
        // The newest channel message came in on the keyboard (its note-off
        // at 0.95 s), on channel 1.
        let s = auto_stream(&m.monitor);
        let keys = m
            .monitor
            .ports()
            .iter()
            .find(|p| p.name == "Keystep 37")
            .unwrap()
            .id;
        assert_eq!(s, (keys, Dir::In));
        assert_eq!(auto_channel(&m.monitor, s), 0);
        let d = data(&m, s, 0, 1.0);
        // E4 and G4 still down (C4 let go), the knob learned.
        assert_eq!(d.held.down().collect::<Vec<_>>(), [64, 67]);
        assert_eq!(d.learned, [(74, "Cutoff (Filter, Drums)".to_string())]);
        assert_eq!(d.went, "not played: Learn only");
        // The clock on the 5-pin jacks, which the tempo follows.
        let c = d.clock.as_ref().unwrap();
        assert!(c.followed && c.name == "MIDI" && c.dir == Dir::In);
        // The drum machine's channel 10: pad A1 let go, H2's note still
        // down.
        let drums = data(&m, (mpfree_midi::DIN, Dir::In), 9, 1.0);
        assert_eq!(drums.held.down().collect::<Vec<_>>(), [20]);
        assert_eq!(drums.went, "Drums, pad A1");
        // MIDI Out: the Bass note ended, so nothing is still on.
        assert!(d.still.is_empty());
        assert!(!can_end(&d));
    }

    #[test]
    fn the_keyboard_moves_to_show_the_notes_held() {
        let mut h = Held::new();
        assert_eq!(keyboard_base(&h), BASE);
        let on = |n| Msg::NoteOn {
            ch: 0,
            note: n,
            vel: 90,
        };
        h.take(&on(60), 0.0, To::Nothing);
        assert_eq!(keyboard_base(&h), BASE);
        h.take(&on(100), 0.0, To::Nothing);
        assert_eq!(keyboard_base(&h), 60);
        // From the C below the lowest held, at most C7.
        let mut low = Held::new();
        low.take(&on(40), 0.0, To::Nothing);
        assert_eq!(keyboard_base(&low), 36);
        let mut high = Held::new();
        high.take(&on(125), 0.0, To::Nothing);
        assert_eq!(keyboard_base(&high), 96);
    }

    use mpfree_midi::Msg;
}
