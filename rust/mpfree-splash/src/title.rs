//! The title's letters, drawn as smooth rounded strokes in the spirit of
//! Ubuntu Mono (round caps, humanist curves, a narrow zero, a footed "r" and
//! "1"). These outlines are our own, drawn for MPFree from simple lines and
//! arcs; no font file is copied, so the Ubuntu Font Licence does not apply.
//!
//! Units: a character is 6.5 wide; capitals run from y = 0 (top) to y = 10
//! (baseline), lower case from y = 3. The renderer scales these to pixels.

use std::f32::consts::PI;

/// Horizontal advance of one character, in glyph units.
pub const ADVANCE: f32 = 6.5;
/// Cap height, in glyph units.
pub const CAP: f32 = 10.0;
/// Space between two lines, in glyph units.
pub const LINE_GAP: f32 = 4.0;
/// Half the stroke width, in glyph units.
pub const HALF_STROKE: f32 = 0.78;

type Seg = ((f32, f32), (f32, f32));

fn poly(points: &[(f32, f32)], out: &mut Vec<Seg>) {
    for pair in points.windows(2) {
        out.push((pair[0], pair[1]));
    }
}

/// An elliptical arc from `from` to `to` degrees (counter-clockwise, 0 = right,
/// 90 = up, as on paper; y grows downwards in glyph units).
fn arc(cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32, out: &mut Vec<Seg>) {
    let steps = (((to - from).abs() / 12.0).ceil() as usize).max(2);
    let pts: Vec<(f32, f32)> = (0..=steps)
        .map(|i| {
            let t = (from + (to - from) * i as f32 / steps as f32) * PI / 180.0;
            (cx + rx * t.cos(), cy - ry * t.sin())
        })
        .collect();
    poly(&pts, out);
}

/// The strokes of one character, or None if the title alphabet lacks it.
pub fn strokes(c: char) -> Option<Vec<Seg>> {
    let mut s = Vec::new();
    match c {
        ' ' => {}
        'M' => poly(&[(1.0, 10.0), (1.3, 0.2), (3.25, 6.4), (5.2, 0.2), (5.5, 10.0)], &mut s),
        'P' => {
            poly(&[(1.3, 10.0), (1.3, 0.2), (3.3, 0.2)], &mut s);
            arc(3.3, 2.85, 2.0, 2.65, 90.0, -90.0, &mut s);
            poly(&[(3.3, 5.5), (1.3, 5.5)], &mut s);
        }
        'C' => arc(3.5, 5.1, 2.5, 4.9, 48.0, 312.0, &mut s),
        'F' => {
            poly(&[(1.5, 10.0), (1.5, 0.2), (5.4, 0.2)], &mut s);
            poly(&[(1.5, 4.9), (4.7, 4.9)], &mut s);
        }
        'r' => {
            poly(&[(0.9, 3.2), (2.3, 3.2), (2.3, 10.0)], &mut s);
            poly(&[(0.9, 10.0), (4.4, 10.0)], &mut s);
            arc(4.7, 6.4, 2.4, 3.0, 180.0, 85.0, &mut s);
            poly(&[(4.9, 3.4), (5.6, 3.6)], &mut s);
        }
        'e' => {
            poly(&[(1.1, 6.6), (5.6, 6.6)], &mut s);
            arc(3.35, 6.6, 2.25, 3.4, 0.0, 180.0, &mut s);
            arc(3.35, 6.6, 2.25, 3.4, 180.0, 305.0, &mut s);
        }
        'O' => arc(3.25, 5.1, 2.35, 4.9, 0.0, 360.0, &mut s),
        'S' => {
            arc(3.3, 2.75, 2.05, 2.55, 30.0, 270.0, &mut s);
            arc(3.3, 7.6, 2.25, 2.35, 90.0, -155.0, &mut s);
        }
        'v' => poly(&[(1.0, 3.2), (3.25, 10.0), (5.5, 3.2)], &mut s),
        // Narrower than the O, so the two are told apart without a dot or slash.
        '0' => arc(3.25, 5.1, 1.85, 4.9, 0.0, 360.0, &mut s),
        '.' => poly(&[(3.25, 9.2), (3.25, 9.3)], &mut s),
        '1' => {
            poly(&[(1.6, 1.9), (3.5, 0.2), (3.5, 10.0)], &mut s);
            poly(&[(1.4, 10.0), (5.4, 10.0)], &mut s);
        }
        _ => return None,
    }
    Some(s)
}

fn distance(p: (f32, f32), seg: &Seg) -> f32 {
    let ((ax, ay), (bx, by)) = *seg;
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - ax) * dx + (p.1 - ay) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (ax + t * dx - p.0, ay + t * dy - p.1);
    (qx * qx + qy * qy).sqrt()
}

/// The title rasterised at `cap` pixels per cap height: for each pixel, how
/// much is letter (0..1) and how much is outline (0..1, under the letter).
pub struct TitleMask {
    pub width: usize,
    pub height: usize,
    pub ink: Vec<f32>,
    pub halo: Vec<f32>,
}

/// Lays `lines` out centred, one above the other, at `cap` pixels per cap height.
pub fn rasterise(lines: &[&str], cap: f32) -> TitleMask {
    let scale = cap / CAP;
    let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
    // A margin of one outline width on every side.
    let outline = (0.9f32).max(1.2 / scale);
    let margin = HALF_STROKE + outline + 0.2;
    let units_w = longest * ADVANCE + 2.0 * margin;
    let units_h = lines.len() as f32 * CAP + (lines.len().saturating_sub(1)) as f32 * LINE_GAP + 2.0 * margin;
    let width = (units_w * scale).ceil() as usize;
    let height = (units_h * scale).ceil() as usize;

    // All strokes, placed.
    let mut glyphs: Vec<(f32, Vec<Seg>)> = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let n = line.chars().count() as f32;
        let left = margin + (longest - n) * ADVANCE / 2.0;
        let top = margin + li as f32 * (CAP + LINE_GAP);
        for (ci, ch) in line.chars().enumerate() {
            let x0 = left + ci as f32 * ADVANCE;
            let segs = strokes(ch).unwrap_or_default();
            let placed = segs.iter().map(|&((ax, ay), (bx, by))| ((ax + x0, ay + top), (bx + x0, by + top))).collect();
            glyphs.push((x0, placed));
        }
    }

    let mut ink = vec![0.0; width * height];
    let mut halo = vec![0.0; width * height];
    // Half a pixel, in glyph units: the width of the anti-aliased edge.
    let soft = 0.5 / scale;
    for py in 0..height {
        for px in 0..width {
            let p = ((px as f32 + 0.5) / scale, (py as f32 + 0.5) / scale);
            let mut d = f32::MAX;
            for (x0, segs) in &glyphs {
                if p.0 < x0 - 2.0 || p.0 > x0 + ADVANCE + 2.0 {
                    continue;
                }
                for seg in segs {
                    d = d.min(distance(p, seg));
                }
            }
            let cover = |edge: f32| ((edge - d) / (2.0 * soft) + 0.5).clamp(0.0, 1.0);
            ink[py * width + px] = cover(HALF_STROKE);
            halo[py * width + px] = cover(HALF_STROKE + outline);
        }
    }
    TitleMask { width, height, ink, halo }
}

/// Size in pixels of `lines` at `cap` pixels per cap height, without rasterising.
pub fn size(lines: &[&str], cap: f32) -> (usize, usize) {
    let scale = cap / CAP;
    let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
    let outline = (0.9f32).max(1.2 / scale);
    let margin = HALF_STROKE + outline + 0.2;
    let w = longest * ADVANCE + 2.0 * margin;
    let h = lines.len() as f32 * CAP + (lines.len().saturating_sub(1)) as f32 * LINE_GAP + 2.0 * margin;
    ((w * scale).ceil() as usize, (h * scale).ceil() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_title_character_has_strokes() {
        for c in "MPCFree OS v0.1".chars() {
            let s = strokes(c).unwrap_or_else(|| panic!("no strokes for {c:?}"));
            assert_eq!(s.is_empty(), c == ' ', "{c:?}");
            for ((ax, ay), (bx, by)) in s {
                for (x, y) in [(ax, ay), (bx, by)] {
                    assert!((0.0..=ADVANCE).contains(&x) && (0.0..=CAP).contains(&y), "{c:?} point ({x}, {y})");
                }
            }
        }
        assert!(strokes('Z').is_none());
    }

    #[test]
    fn letters_are_solid_where_strokes_are() {
        let m = rasterise(&["MPCFree"], 40.0);
        assert_eq!((m.width, m.height), size(&["MPCFree"], 40.0));
        // The middle of M's left leg is fully letter; the gap inside the C is not.
        let scale = 4.0;
        let margin = HALF_STROKE + 0.9 + 0.2;
        let at = |ux: f32, uy: f32| {
            let (x, y) = (((ux + margin) * scale) as usize, ((uy + margin) * scale) as usize);
            m.ink[y * m.width + x]
        };
        assert!(at(1.15, 5.0) > 0.99);
        assert!(at(2.0 * ADVANCE + 3.5, 5.1) < 0.01, "inside the C");
        assert!(m.halo.iter().zip(&m.ink).all(|(h, i)| h >= i));
    }
}
