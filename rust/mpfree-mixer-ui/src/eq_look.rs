//! The Mix tab's EQ look (layout spec section 3a, Jay's EQ boards, 15 Nov):
//! a row a strip, as on Effects, with the strip's first parametric EQ
//! drawn as its response curve and a dot a band. A tap on a curve picks
//! that EQ (and the band under the finger) for the Q-Links, a tap again
//! opens it in the chain editor, and a dot dragged past the lift moves its
//! band (sideways its frequency, up and down its gain). A strip without
//! one shows "+ EQ", which adds one in its first free slot.
//!
//! What is drawn comes from [`mpfree_fx::Eq::response_db`] fed the
//! insert's values ([`crate::InsertView::eq`]), so it is what plays.

use mpfree_fx::ids::eq as ids;
use mpfree_fx::mpfree_node::Node;
use mpfree_ui::{Color, Rect};

/// The graph's axes.
const F_LO: f32 = 20.0;
const F_HI: f32 = 20_000.0;
pub const DB: f32 = 18.0;
/// Points across a row's curve.
pub const K: usize = 130;
/// Pixels a finger moves on a dot before it follows (spec 4h).
pub const LIFT: i32 = 12;
/// How far from a dot's middle a finger still takes it.
pub const HIT: f32 = 22.0;
/// The space a row keeps right of its curve for what the EQ is.
const SUMMARY_W: i32 = 206;

/// A band's colour: as on the EQ's page (bands 1 to 4 wear the Q-Links'
/// colours).
pub const BAND_COLORS: [Color; 8] = [
    Color::rgb(0x6EA8FE),
    Color::rgb(0x3DDC97),
    Color::rgb(0xFFC145),
    Color::rgb(0xD46CFF),
    Color::rgb(0xFF8A65),
    Color::rgb(0x4FC3C9),
    Color::rgb(0xF06292),
    Color::rgb(0xAED581),
];
pub const CUT_COLOR: Color = Color::rgb(0xB8C2D6);

/// A dot: band 0 to 7, or a cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    Band(usize),
    Low,
    High,
}

impl Dot {
    pub fn label(self) -> String {
        match self {
            Dot::Band(b) => (b + 1).to_string(),
            Dot::Low => "LC".into(),
            Dot::High => "HC".into(),
        }
    }
    pub fn name(self) -> String {
        match self {
            Dot::Band(b) => format!("Band {}", b + 1),
            Dot::Low => "Low cut".into(),
            Dot::High => "High cut".into(),
        }
    }
    pub fn color(self) -> Color {
        match self {
            Dot::Band(b) => BAND_COLORS[b % BAND_COLORS.len()],
            _ => CUT_COLOR,
        }
    }
    /// The parameter picking the dot focuses: what its dot moves up and
    /// down, else across (the chain editor's focus follows it).
    pub fn main_id(self) -> u16 {
        match self {
            Dot::Band(b) => ids::gain(b),
            Dot::Low => ids::LOW_CUT,
            Dot::High => ids::HIGH_CUT,
        }
    }
}

/// A row's parts: the strip's name, the curve's box and the summary.
pub fn row_parts(row: Rect, name_w: i32) -> (Rect, Rect, Rect) {
    let name = row.take_left(name_w).0;
    let rest = Rect::new(row.x + name_w + 12, row.y, row.w - name_w - 12, row.h);
    let w = (rest.w - SUMMARY_W).max(120);
    let graph = Rect::new(rest.x, rest.y, w, rest.h);
    let summary = Rect::new(
        graph.right() + 10,
        row.y,
        row.right() - graph.right() - 10,
        row.h,
    );
    (name, graph, summary)
}

/// Where a frequency sits across the graph, 0 to 1, and back.
pub fn freq_x(hz: f32) -> f32 {
    ((hz.max(F_LO) / F_LO).ln() / (F_HI / F_LO).ln()).clamp(0.0, 1.0)
}
pub fn x_freq(x: f32) -> f32 {
    F_LO * (F_HI / F_LO).powf(x.clamp(0.0, 1.0))
}

/// The plot inside a row's box: a little room above and below.
pub fn plot(graph: Rect) -> Rect {
    graph.inset_xy(2, 4)
}

/// A dB value's height in plot `p` (clamped to the plot).
pub fn db_y(p: Rect, db: f32) -> f32 {
    let up = (0.5 + db / (2.0 * DB)).clamp(0.0, 1.0);
    p.bottom() as f32 - up * p.h as f32
}

fn v(vals: &[f32], id: u16) -> f32 {
    vals.get(id as usize).copied().unwrap_or(0.0)
}

/// The dots an EQ shows: its cuts when on, and its bands in use.
pub fn dots(vals: &[f32]) -> Vec<Dot> {
    let mut out = Vec::new();
    if v(vals, ids::LOW_SLOPE) > 0.0 {
        out.push(Dot::Low);
    }
    for b in 0..ids::BANDS {
        if v(vals, ids::band_use(b)) != ids::USE_UNUSED {
            out.push(Dot::Band(b));
        }
    }
    if v(vals, ids::HIGH_SLOPE) > 0.0 {
        out.push(Dot::High);
    }
    out
}

/// Band `b` is heard (a cut with a dot always is).
pub fn heard(vals: &[f32], d: Dot) -> bool {
    match d {
        Dot::Band(b) => v(vals, ids::band_use(b)) == ids::USE_ON,
        _ => true,
    }
}

const NOTCH: f32 = 3.0;

/// The EQ's response in dB at `K` points from 20 Hz to 20 kHz.
pub fn curve(vals: &[f32]) -> Vec<f32> {
    let mut e = mpfree_fx::Eq::new();
    for (id, &x) in vals.iter().enumerate() {
        e.set_param(id as u16, x);
    }
    let hz: Vec<f32> = (0..K).map(|k| x_freq(k as f32 / (K - 1) as f32)).collect();
    let mut db = vec![0.0; K];
    e.response_db(&hz, &mut db);
    db
}

/// The curve's value nearest `hz`.
fn curve_at(curve: &[f32], hz: f32) -> f32 {
    if curve.is_empty() {
        return 0.0;
    }
    let k = (freq_x(hz) * (curve.len() - 1) as f32).round() as usize;
    curve[k.min(curve.len() - 1)]
}

/// Dot `d`'s middle in plot `p`: a band at its frequency and gain (a
/// notch on the 0 dB line), a cut on the curve at its frequency.
pub fn dot_at(vals: &[f32], curve: &[f32], p: Rect, d: Dot) -> (f32, f32) {
    let x = |hz: f32| p.x as f32 + freq_x(hz) * p.w as f32;
    match d {
        Dot::Band(b) => {
            let hz = v(vals, ids::freq(b));
            let db = if v(vals, ids::band_type(b)) == NOTCH {
                0.0
            } else {
                v(vals, ids::gain(b))
            };
            (x(hz), db_y(p, db))
        }
        Dot::Low | Dot::High => {
            let hz = v(
                vals,
                if d == Dot::Low {
                    ids::LOW_CUT
                } else {
                    ids::HIGH_CUT
                },
            );
            (x(hz), db_y(p, curve_at(curve, hz)))
        }
    }
}

/// The dot under a finger at `(x, y)`, the nearest within reach.
pub fn hit(vals: &[f32], curve: &[f32], p: Rect, x: i32, y: i32) -> Option<Dot> {
    dots(vals)
        .into_iter()
        .map(|d| {
            let (dx, dy) = dot_at(vals, curve, p, d);
            (d, (dx - x as f32).hypot(dy - y as f32))
        })
        .filter(|&(_, dist)| dist <= HIT)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(d, _)| d)
}

/// What dragging dot `d` to `(x, y)` in plot `p` sets: (id, value).
pub fn drag(vals: &[f32], p: Rect, d: Dot, x: i32, y: i32) -> Vec<(u16, f32)> {
    let fx = (x - p.x) as f32 / p.w.max(1) as f32;
    let fy = (p.bottom() - y) as f32 / p.h.max(1) as f32;
    let hz = x_freq(fx).round();
    match d {
        Dot::Low => vec![(ids::LOW_CUT, hz.clamp(20.0, 2_000.0))],
        Dot::High => vec![(ids::HIGH_CUT, hz.clamp(1_000.0, 20_000.0))],
        Dot::Band(b) => {
            let mut out = vec![(ids::freq(b), hz)];
            if v(vals, ids::band_type(b)) != NOTCH {
                let db = ((fy - 0.5) * 2.0 * DB).clamp(-DB, DB);
                out.push((ids::gain(b), (db * 2.0).round() / 2.0));
            }
            out
        }
    }
}

/// "80 Hz", "1.2 kHz".
pub fn hz_text(hz: f32) -> String {
    if hz >= 1_000.0 {
        let k = hz / 1_000.0;
        if k >= 10.0 {
            format!("{k:.0} kHz")
        } else {
            format!("{k:.1} kHz")
        }
    } else {
        format!("{hz:.0} Hz")
    }
}

/// The two lines beside a curve: where the EQ is and how many bands it
/// has, then its cuts (or "Bypassed" while the insert is off).
pub fn summary(vals: &[f32], slot: usize, on: bool) -> (String, String) {
    let bands = (0..ids::BANDS)
        .filter(|&b| v(vals, ids::band_use(b)) != ids::USE_UNUSED)
        .count();
    let first = format!(
        "Slot {} · {bands} band{}",
        slot + 1,
        if bands == 1 { "" } else { "s" }
    );
    if !on {
        return (first, "Bypassed".into());
    }
    let cut = |name: &str, f: u16, s: u16| {
        let slope = v(vals, s);
        (slope > 0.0).then(|| {
            format!(
                "{name} {}, {} dB",
                hz_text(v(vals, f)),
                if slope >= 2.0 { 24 } else { 12 }
            )
        })
    };
    let cuts: Vec<String> = [
        cut("Low cut", ids::LOW_CUT, ids::LOW_SLOPE),
        cut("High cut", ids::HIGH_CUT, ids::HIGH_SLOPE),
    ]
    .into_iter()
    .flatten()
    .collect();
    let second = match cuts.len() {
        0 => "Cuts off".to_string(),
        1 => cuts[0].clone(),
        _ => format!(
            "Cuts {} and {}",
            hz_text(v(vals, ids::LOW_CUT)),
            hz_text(v(vals, ids::HIGH_CUT))
        ),
    };
    (first, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq_vals(set: &[(u16, f32)]) -> Vec<f32> {
        let mut e = mpfree_fx::Eq::new();
        for &(id, x) in set {
            e.set_param(id, x);
        }
        e.params().iter().map(|p| e.param(p.id)).collect()
    }

    #[test]
    fn dots_sit_where_their_bands_are_and_a_drag_moves_them() {
        let vals = eq_vals(&[
            (ids::LOW_SLOPE, 1.0),
            (ids::freq(1), 1_000.0),
            (ids::gain(1), 6.0),
        ]);
        let c = curve(&vals);
        assert_eq!(
            dots(&vals),
            [
                Dot::Low,
                Dot::Band(0),
                Dot::Band(1),
                Dot::Band(2),
                Dot::Band(3)
            ]
        );
        let p = Rect::new(0, 0, 600, 44);
        let (x, y) = dot_at(&vals, &c, p, Dot::Band(1));
        let want = 600.0 * 50f32.ln() / 1000f32.ln();
        assert!((x - want).abs() < 1.0, "1 kHz on a log scale: {x}");
        assert!((y - (44.0 - (0.5 + 6.0 / 36.0) * 44.0)).abs() < 0.5);
        assert_eq!(
            hit(&vals, &c, p, x as i32 + 3, y as i32),
            Some(Dot::Band(1))
        );
        let set = drag(&vals, p, Dot::Band(1), 450, 0);
        assert_eq!(set[1], (ids::gain(1), 18.0));
        assert!(set[0].1 > 3_000.0, "{set:?}");
    }

    #[test]
    fn the_summary_says_where_and_what() {
        let vals = eq_vals(&[(ids::LOW_SLOPE, 2.0), (ids::LOW_CUT, 35.0)]);
        assert_eq!(
            summary(&vals, 1, true),
            (
                "Slot 2 · 4 bands".to_string(),
                "Low cut 35 Hz, 24 dB".to_string()
            )
        );
        assert_eq!(summary(&vals, 0, false).1, "Bypassed");
    }
}
