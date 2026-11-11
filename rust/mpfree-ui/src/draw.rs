//! Small drawing pieces every panel shares, so the same look is written
//! once: a card with its heading, a pill, a dot, a speaker (mute) icon, a
//! segmented level meter and a ring of dots for a knob. They are the
//! Play page mockups' own helpers (Jay approved those pictures, 10:05Z),
//! moved here so the app's widgets draw exactly what the mockups drew.
//!
//! Colours come from the theme, except the few fixed ones the mockups use
//! for knobs and kinds of sound ([`KNOB`], [`sound_color`]).

use crate::color::Color;
use crate::geom::Rect;
use crate::text::{Align, FontId};
use crate::theme::Theme;
use crate::ui::PaintCx;

/// The four Q-Link knobs' colours, knob 1 first: blue, green, yellow, pink.
pub const KNOB: [Color; 4] = [
    Color::rgb(0x6EA8FE),
    Color::rgb(0x3DDC97),
    Color::rgb(0xFFC145),
    Color::rgb(0xD46CFF),
];

/// Kinds of sound, as the mockups colour them.
pub const KICK: Color = Color::rgb(0xFF6B6B);
pub const SNARE: Color = Color::rgb(0xFFC145);
pub const HAT: Color = Color::rgb(0x3DDC97);
pub const PERC: Color = Color::rgb(0x4FD1E8);
pub const BASS: Color = Color::rgb(0x6EA8FE);
pub const CHOP: Color = Color::rgb(0xD46CFF);
pub const VOX: Color = Color::rgb(0xFF8FD8);
pub const FX: Color = Color::rgb(0xA0A8B8);

/// The kinds of sound: the words a name of each has, its colour and the
/// group it belongs to (what a loop layer of them is called).
const KINDS: [(&[&str], Color, &str); 8] = [
    (&["kick", "bd", "808", "boom"], KICK, "Drums"),
    (&["snare", "sd", "rim", "clap", "snap"], SNARE, "Drums"),
    (&["hat", "hh", "shaker", "cabasa", "tamb"], HAT, "Drums"),
    (
        &[
            "tom", "conga", "bongo", "perc", "crash", "ride", "cymbal", "cow", "clave", "wood",
            "block",
        ],
        PERC,
        "Drums",
    ),
    (&["bass", "sub"], BASS, "Bass"),
    (
        &["chop", "stab", "chord", "keys", "piano", "synth", "lead"],
        CHOP,
        "Chop",
    ),
    (&["vox", "vocal", "voice", "choir"], VOX, "Vox"),
    (&["fx", "noise", "riser", "sweep", "hit"], FX, "FX"),
];

fn kind(name: &str) -> Option<&'static (&'static [&'static str], Color, &'static str)> {
    let n = name.to_ascii_lowercase();
    KINDS
        .iter()
        .find(|(words, _, _)| words.iter().any(|w| n.contains(w)))
}

/// The colour of a sound named `name` ("Kick 2", "Hat op", "Chop 1"), by
/// its kind, or None when the name says nothing about it ("C3", "Pad 7").
pub fn sound_color(name: &str) -> Option<Color> {
    kind(name).map(|k| k.1)
}

/// The group a sound named `name` belongs to: "Drums" for kicks, snares,
/// hats and percussion, else "Bass", "Chop", "Vox" or "FX"; None when the
/// name says nothing about it.
pub fn sound_group(name: &str) -> Option<&'static str> {
    kind(name).map(|k| k.2)
}

/// Text sizes the panels use (the mockups' own).
pub const HEAD_PX: f32 = 12.5;

/// A dot of radius `r` centred on (x, y).
pub fn dot(cx: &mut PaintCx, x: i32, y: i32, r: i32, c: Color) {
    cx.canvas
        .fill_round_rect(Rect::new(x - r, y - r, 2 * r, 2 * r), r, c);
}

/// A panel with a small heading in its top corners: the title in capitals
/// on the left, `right` on the right. Returns the area under the heading.
pub fn card(cx: &mut PaintCx, r: Rect, title: &str, right: &str) -> Rect {
    let t = cx.theme.clone();
    cx.panel(r);
    let (h, body) = card_head(r);
    cx.text(
        FontId::Bold,
        HEAD_PX,
        h,
        Align::Left,
        t.text_dim,
        &title.to_uppercase(),
    );
    if !right.is_empty() {
        cx.text(FontId::Sans, HEAD_PX, h, Align::Right, t.text_faint, right);
    }
    body
}

/// A card's heading line and the area under it, for a card in `r`.
pub fn card_head(r: Rect) -> (Rect, Rect) {
    let inner = r.inset_xy(12, 10);
    let (h, body) = inner.take_top(18);
    (h, body.take_top(8).1)
}

/// A rounded pill with `text`: filled with `tone`, or raised with a dot of
/// `tone` before the text. Returns its width.
pub fn pill(
    cx: &mut PaintCx,
    x: i32,
    y: i32,
    h: i32,
    text: &str,
    tone: Color,
    filled: bool,
) -> i32 {
    let t = cx.theme.clone();
    let w = pill_width(cx, text, filled);
    let r = Rect::new(x, y, w, h);
    if filled {
        cx.canvas.fill_round_rect(r, h / 2, tone);
        cx.text(FontId::Bold, 14.0, r, Align::Center, t.bg, text);
    } else {
        cx.canvas.fill_round_rect(r, h / 2, t.raised);
        dot(cx, x + 17, y + h / 2, 5, tone);
        let tw = cx.measure(FontId::Bold, 14.0, text);
        cx.text(
            FontId::Bold,
            14.0,
            Rect::new(x + 28, y, tw + 4, h),
            Align::Left,
            t.text,
            text,
        );
    }
    w
}

/// The width [`pill`] takes for `text`.
pub fn pill_width(cx: &mut PaintCx, text: &str, filled: bool) -> i32 {
    cx.measure(FontId::Bold, 14.0, text) + if filled { 28 } else { 40 }
}

/// A speaker about 22 px wide centred on (x, y): sound waves when on, a red
/// cross when muted.
pub fn speaker(cx: &mut PaintCx, x: i32, y: i32, c: Color, muted: bool) {
    let t = cx.theme.clone();
    let body = if muted { t.text_faint } else { c };
    cx.canvas.fill_rect(Rect::new(x - 10, y - 3, 4, 6), body);
    cx.canvas.fill_rect(Rect::new(x - 6, y - 4, 2, 8), body);
    cx.canvas.fill_rect(Rect::new(x - 4, y - 6, 2, 12), body);
    if muted {
        cx.text(
            FontId::Bold,
            14.0,
            Rect::new(x, y - 8, 12, 16),
            Align::Left,
            t.bad,
            "×",
        );
    } else {
        for (rad, n) in [(5.0f32, 3), (9.0, 5)] {
            for k in 0..n {
                let a = (-40.0 + 80.0 * k as f32 / (n - 1) as f32).to_radians();
                let px = x - 2 + (rad * a.cos()).round() as i32;
                let py = y + (rad * a.sin()).round() as i32;
                cx.canvas.fill_rect(Rect::new(px, py, 2, 2), body);
            }
        }
    }
}

/// The square a speaker centred on (x, y) answers taps in.
pub fn speaker_hit(x: i32, y: i32) -> Rect {
    Rect::new(x - 16, y - 16, 32, 32)
}

/// Where a level in dB sits along a meter: -48 dB at 0, +6 dB at 1.
pub fn meter_frac(db: f32) -> f32 {
    ((db + 48.0) / 54.0).clamp(0.0, 1.0)
}

/// The dB at a place along a meter (the inverse of [`meter_frac`]).
pub fn meter_db(frac: f32) -> f32 {
    frac.clamp(0.0, 1.0) * 54.0 - 48.0
}

/// A meter segment's colour at `db`: red near full scale, yellow in the
/// top 12 dB, green under.
pub fn segment_color(t: &Theme, db: f32) -> Color {
    if db > -3.0 {
        t.bad
    } else if db > -12.0 {
        t.warn
    } else {
        t.good
    }
}

/// One channel of a level meter in segments, bottom to top (or left to
/// right), lit up to `db`, with a line at `peak`.
pub fn meter_bar(cx: &mut PaintCx, r: Rect, db: f32, peak: f32, vertical: bool) {
    let t = cx.theme.clone();
    let len = if vertical { r.h } else { r.w };
    let seg = 5;
    let step = seg + 2;
    let lit = (meter_frac(db) * len as f32) as i32;
    let mut p = 0;
    while p + seg <= len {
        let f = (p + seg / 2) as f32 / len as f32;
        let c = if p < lit {
            segment_color(&t, meter_db(f))
        } else {
            t.raised
        };
        let sr = if vertical {
            Rect::new(r.x, r.bottom() - p - seg, r.w, seg)
        } else {
            Rect::new(r.x + p, r.y, seg, r.h)
        };
        cx.canvas.fill_rect(sr, c);
        p += step;
    }
    if peak > -48.0 {
        let pk = (meter_frac(peak) * len as f32) as i32;
        let pr = if vertical {
            Rect::new(r.x, r.bottom() - pk, r.w, 2)
        } else {
            Rect::new(r.x + pk, r.y, 2, r.h)
        };
        cx.canvas.fill_rect(pr, segment_color(&t, peak));
    }
}

/// A knob's ring: 33 dots over 270 degrees, open at the bottom, centred on
/// (x, y) with radius `rad`, lit as far as `v` (0 to 1; from the top both
/// ways when `bipolar`).
pub fn ring(cx: &mut PaintCx, x: i32, y: i32, rad: f32, v: f32, bipolar: bool, lit: Color) {
    let t = cx.theme.clone();
    let n = 33;
    let v = v.clamp(0.0, 1.0);
    for k in 0..n {
        let f = k as f32 / (n - 1) as f32;
        let a = (135.0 + 270.0 * f).to_radians();
        let px = x + (rad * a.cos()).round() as i32;
        let py = y + (rad * a.sin()).round() as i32;
        let on = if bipolar {
            (f >= 0.5 && f <= v) || (f <= 0.5 && f >= v)
        } else {
            f <= v
        };
        dot(
            cx,
            px,
            py,
            if on { 3 } else { 2 },
            if on { lit } else { t.outline },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_coloured_by_kind() {
        assert_eq!(sound_color("Kick 2"), Some(KICK));
        assert_eq!(sound_color("Hat op"), Some(HAT));
        assert_eq!(sound_color("Closed Hat"), Some(HAT));
        assert_eq!(sound_color("Rim"), Some(SNARE));
        assert_eq!(sound_color("Sub Hit"), Some(BASS));
        assert_eq!(sound_color("Chop 1"), Some(CHOP));
        assert_eq!(sound_color("Tom Lo"), Some(PERC));
        assert_eq!(sound_color("C3"), None);
        assert_eq!(sound_group("Hat op"), Some("Drums"));
        assert_eq!(sound_group("Chop 2"), Some("Chop"));
        assert_eq!(sound_group("Pad 7"), None);
    }

    #[test]
    fn the_meter_scale_goes_both_ways() {
        for db in [-48.0f32, -24.0, -1.5, 0.0, 6.0] {
            assert!((meter_db(meter_frac(db)) - db).abs() < 1e-4);
        }
        assert_eq!(meter_frac(-90.0), 0.0);
    }
}
