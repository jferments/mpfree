//! A heavy, squared sci-fi display face for the title, in the spirit of
//! display fonts like "Antares" and "Jupiter": thick strokes, square counters
//! and 45-degree cut corners. The shapes are our own, drawn for MPFree as
//! polygons; no font file is used or copied.
//!
//! Units: capitals run from y = 0 (top) to y = 12 (baseline); lower case
//! from y = 4. Each glyph is a set of closed outlines filled with the
//! even-odd rule, so an inner outline cuts a counter (the hole in an O).

pub const CAP: f32 = 12.0;
/// Space between letters, in glyph units.
pub const TRACKING: f32 = 1.6;
/// Space between two lines, in glyph units.
pub const LINE_GAP: f32 = 4.0;

pub type Contour = Vec<(f32, f32)>;

pub struct Glyph {
    pub width: f32,
    pub contours: Vec<Contour>,
}

/// A rectangle with its corners cut at 45 degrees: `cuts` are the cut sizes
/// for the top left, top right, bottom right and bottom left corners.
fn cut_rect(x0: f32, y0: f32, x1: f32, y1: f32, cuts: [f32; 4]) -> Contour {
    let [tl, tr, br, bl] = cuts;
    let mut c = Vec::new();
    let mut corner = |a: (f32, f32), b: (f32, f32), cut: f32| {
        if cut > 0.0 {
            c.push(a);
            c.push(b);
        } else {
            c.push(a);
        }
    };
    corner((x0, y0 + tl), (x0 + tl, y0), tl);
    corner((x1 - tr, y0), (x1, y0 + tr), tr);
    corner((x1, y1 - br), (x1 - br, y1), br);
    corner((x0 + bl, y1), (x0, y1 - bl), bl);
    c.dedup();
    c
}

/// The glyph for `c`, or None if the title alphabet lacks it.
pub fn glyph(c: char) -> Option<Glyph> {
    let g = |width: f32, contours: Vec<Contour>| Some(Glyph { width, contours });
    match c {
        ' ' => g(4.5, vec![]),
        'M' => g(
            12.0,
            vec![vec![
                (0.0, 12.0),
                (0.0, 2.5),
                (2.5, 0.0),
                (4.2, 0.0),
                (6.0, 3.6),
                (7.8, 0.0),
                (9.5, 0.0),
                (12.0, 2.5),
                (12.0, 12.0),
                (9.2, 12.0),
                (9.2, 5.4),
                (7.1, 9.0),
                (4.9, 9.0),
                (2.8, 5.4),
                (2.8, 12.0),
            ]],
        ),
        'P' => g(
            9.5,
            vec![
                vec![
                    (0.0, 12.0),
                    (0.0, 3.0),
                    (3.0, 0.0),
                    (9.5, 0.0),
                    (9.5, 6.6),
                    (7.6, 8.5),
                    (2.8, 8.5),
                    (2.8, 12.0),
                ],
                cut_rect(2.8, 2.6, 6.7, 5.9, [0.9, 0.0, 0.0, 0.0]),
            ],
        ),
        'C' => g(
            9.5,
            vec![vec![
                (3.0, 0.0),
                (9.5, 0.0),
                (9.5, 2.6),
                (3.9, 2.6),
                (2.8, 3.7),
                (2.8, 8.3),
                (3.9, 9.4),
                (9.5, 9.4),
                (9.5, 12.0),
                (3.0, 12.0),
                (0.0, 9.0),
                (0.0, 3.0),
            ]],
        ),
        'F' => g(
            9.0,
            vec![vec![
                (0.0, 12.0),
                (0.0, 3.0),
                (3.0, 0.0),
                (9.0, 0.0),
                (9.0, 2.6),
                (2.8, 2.6),
                (2.8, 4.8),
                (7.6, 4.8),
                (7.6, 7.3),
                (2.8, 7.3),
                (2.8, 12.0),
            ]],
        ),
        'O' => g(
            10.0,
            vec![
                cut_rect(0.0, 0.0, 10.0, 12.0, [3.0, 0.0, 3.0, 0.0]),
                cut_rect(2.8, 2.6, 7.2, 9.4, [1.1, 0.0, 1.1, 0.0]),
            ],
        ),
        // Cut the other way from the O, and narrower, so the two read apart.
        '0' => g(
            9.0,
            vec![
                cut_rect(0.0, 0.0, 9.0, 12.0, [0.0, 3.0, 0.0, 3.0]),
                cut_rect(2.8, 2.6, 6.2, 9.4, [0.0, 1.1, 0.0, 1.1]),
            ],
        ),
        'S' => g(
            9.5,
            vec![vec![
                (3.0, 0.0),
                (9.5, 0.0),
                (9.5, 2.6),
                (2.8, 2.6),
                (2.8, 4.7),
                (9.5, 4.7),
                (9.5, 9.0),
                (6.5, 12.0),
                (0.0, 12.0),
                (0.0, 9.4),
                (6.7, 9.4),
                (6.7, 7.3),
                (0.0, 7.3),
                (0.0, 3.0),
            ]],
        ),
        'r' => g(
            8.0,
            vec![vec![
                (0.0, 12.0),
                (0.0, 6.5),
                (2.5, 4.0),
                (8.0, 4.0),
                (8.0, 6.4),
                (2.8, 6.4),
                (2.8, 12.0),
            ]],
        ),
        'e' => g(
            8.5,
            vec![
                vec![
                    (2.5, 4.0),
                    (8.5, 4.0),
                    (8.5, 8.6),
                    (2.8, 8.6),
                    (2.8, 9.8),
                    (8.5, 9.8),
                    (8.5, 12.0),
                    (2.5, 12.0),
                    (0.0, 9.5),
                    (0.0, 6.5),
                ],
                cut_rect(2.8, 6.0, 5.9, 6.9, [0.0; 4]),
            ],
        ),
        'v' => g(
            9.0,
            vec![vec![
                (0.0, 4.0),
                (2.8, 4.0),
                (4.5, 9.0),
                (6.2, 4.0),
                (9.0, 4.0),
                (6.2, 12.0),
                (2.8, 12.0),
            ]],
        ),
        '.' => g(
            3.0,
            vec![cut_rect(0.0, 9.2, 2.8, 12.0, [1.0, 0.0, 0.0, 0.0])],
        ),
        '1' => g(
            6.0,
            vec![vec![
                (0.0, 2.8),
                (2.8, 0.0),
                (6.0, 0.0),
                (6.0, 12.0),
                (3.2, 12.0),
                (3.2, 2.8),
            ]],
        ),
        _ => None,
    }
}

/// Width of one line of text, in glyph units.
pub fn line_width(line: &str) -> f32 {
    let widths: Vec<f32> = line
        .chars()
        .map(|c| glyph(c).map_or(0.0, |g| g.width))
        .collect();
    widths.iter().sum::<f32>() + TRACKING * widths.len().saturating_sub(1) as f32
}

/// Size of `lines` in glyph units, centred one above the other.
pub fn block_size(lines: &[&str]) -> (f32, f32) {
    let w = lines.iter().map(|l| line_width(l)).fold(0.0, f32::max);
    let h = lines.len() as f32 * CAP + lines.len().saturating_sub(1) as f32 * LINE_GAP;
    (w, h)
}

/// Every outline of `lines`, placed in glyph units with (0, 0) at the top left.
pub fn layout(lines: &[&str]) -> Vec<Contour> {
    let (total_w, _) = block_size(lines);
    let mut out = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let mut x = (total_w - line_width(line)) / 2.0;
        let y = li as f32 * (CAP + LINE_GAP);
        for c in line.chars() {
            if let Some(g) = glyph(c) {
                for contour in &g.contours {
                    out.push(contour.iter().map(|&(px, py)| (px + x, py + y)).collect());
                }
                x += g.width + TRACKING;
            }
        }
    }
    out
}

/// Coverage (0..1) of `lines` rasterised `px_per_unit` pixels per glyph unit,
/// with `pad` empty pixels around it, 4x4 samples per pixel.
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub coverage: Vec<f32>,
}

pub fn rasterise(lines: &[&str], px_per_unit: f32, pad: usize) -> Mask {
    let (uw, uh) = block_size(lines);
    let width = (uw * px_per_unit).ceil() as usize + 2 * pad;
    let height = (uh * px_per_unit).ceil() as usize + 2 * pad;
    let contours = layout(lines);
    // Edges with their y range, for a scanline even-odd fill.
    let mut edges: Vec<((f32, f32), (f32, f32))> = Vec::new();
    for c in &contours {
        for i in 0..c.len() {
            let a = c[i];
            let b = c[(i + 1) % c.len()];
            if a.1 != b.1 {
                edges.push((a, b));
            }
        }
    }
    const S: usize = 4;
    let mut coverage = vec![0.0; width * height];
    let mut crossings: Vec<f32> = Vec::new();
    for py in 0..height {
        for sy in 0..S {
            let y = ((py as f32 + (sy as f32 + 0.5) / S as f32) - pad as f32) / px_per_unit;
            crossings.clear();
            for &((ax, ay), (bx, by)) in &edges {
                if (ay <= y) != (by <= y) {
                    crossings.push(ax + (y - ay) / (by - ay) * (bx - ax));
                }
            }
            crossings.sort_by(|a, b| a.total_cmp(b));
            for pair in crossings.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                // Pixel-space span [x0, x1), counted in quarter-pixel samples.
                let x0 = pair[0] * px_per_unit + pad as f32;
                let x1 = pair[1] * px_per_unit + pad as f32;
                let first = ((x0 * S as f32) - 0.5).ceil().max(0.0) as usize;
                let last = ((x1 * S as f32) - 0.5).ceil().min((width * S) as f32) as usize;
                for s in first..last {
                    coverage[py * width + s / S] += 1.0 / (S * S) as f32;
                }
            }
        }
    }
    Mask {
        width,
        height,
        coverage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_title_character_has_a_glyph_inside_its_box() {
        for c in "MPCFree OS v0.1".chars() {
            let g = glyph(c).unwrap_or_else(|| panic!("no glyph for {c:?}"));
            assert_eq!(g.contours.is_empty(), c == ' ', "{c:?}");
            for contour in &g.contours {
                assert!(contour.len() >= 3, "{c:?}");
                for &(x, y) in contour {
                    assert!(
                        (0.0..=g.width).contains(&x) && (0.0..=CAP).contains(&y),
                        "{c:?} point ({x}, {y})"
                    );
                }
            }
        }
        assert!(glyph('Z').is_none());
    }

    #[test]
    fn the_o_has_a_hole_and_solid_walls() {
        let m = rasterise(&["O"], 10.0, 0);
        assert_eq!((m.width, m.height), (100, 120));
        let at =
            |ux: f32, uy: f32| m.coverage[(uy * 10.0) as usize * m.width + (ux * 10.0) as usize];
        assert!(at(1.4, 6.0) > 0.99, "left wall");
        assert!(at(5.0, 6.0) < 0.01, "counter");
        assert!(at(0.2, 0.2) < 0.01, "cut top-left corner");
        assert!(at(9.8, 0.2) > 0.99, "square top-right corner");
        assert!(m.coverage.iter().all(|&v| (0.0..=1.0001).contains(&v)));
    }

    #[test]
    fn lines_are_centred() {
        let (w, h) = block_size(&["MPCFree", "OS v0.1"]);
        assert_eq!(h, 2.0 * CAP + LINE_GAP);
        assert!((w - line_width("MPCFree").max(line_width("OS v0.1"))).abs() < 1e-4);
    }
}
