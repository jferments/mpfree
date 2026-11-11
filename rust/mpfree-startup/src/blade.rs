//! MPFree's own title letters: every stroke is an angular blade that starts
//! with a torn, jagged cut and ends in a knife point, leaning forward. Only
//! the letters the startup titles use are drawn ("MPFree OS", "v0.1").
//!
//! The letters are drawn here as polygons, not taken from a font file, so
//! there is no font licence to carry. A glyph is a few strokes; a stroke is
//! a polyline whose points each carry a width. The first point is the thick
//! end (cut at an angle and torn into teeth, with splinters along it); the
//! last point has width 0, a knife point. Units: baseline 0, cap height 1,
//! x-height about 0.66, y up.

use crate::gfx::{crop, Mask, Rng};

type Pt = (f32, f32, f32);
/// A glyph: its advance and its strokes, each with how torn its start is.
type Glyph = (f32, Vec<(Vec<Pt>, f32)>);

/// The ring of an O (or a narrower 0), opened at the top right so its start
/// is torn and its end is a point.
const RING: [Pt; 10] = [
    (0.48, 1.04, 0.13),
    (0.20, 0.96, 0.15),
    (0.03, 0.66, 0.17),
    (0.04, 0.26, 0.17),
    (0.22, 0.0, 0.15),
    (0.52, -0.02, 0.13),
    (0.76, 0.24, 0.12),
    (0.80, 0.66, 0.11),
    (0.62, 0.98, 0.08),
    (0.30, 1.08, 0.0),
];

/// A glyph's advance and strokes, each with how torn its start is (0..1).
fn glyph(ch: char) -> Option<Glyph> {
    let ring = |sx: f32| RING.iter().map(|&(x, y, w)| (x * sx, y, w)).collect();
    Some(match ch {
        'M' => (
            1.04,
            vec![
                (
                    vec![(0.14, 1.04, 0.19), (0.07, 0.40, 0.17), (0.0, -0.08, 0.0)],
                    1.0,
                ),
                (vec![(0.17, 1.03, 0.16), (0.46, 0.24, 0.04)], 0.5),
                (vec![(0.88, 1.05, 0.16), (0.46, 0.26, 0.03)], 0.5),
                (
                    vec![(0.91, 1.03, 0.19), (0.85, 0.40, 0.17), (0.80, -0.08, 0.0)],
                    1.0,
                ),
            ],
        ),
        'P' => (
            0.86,
            vec![
                (
                    vec![(0.15, 1.03, 0.20), (0.08, 0.40, 0.18), (0.0, -0.10, 0.0)],
                    1.0,
                ),
                (
                    vec![
                        (0.12, 1.0, 0.17),
                        (0.50, 1.0, 0.15),
                        (0.68, 0.88, 0.13),
                        (0.64, 0.62, 0.12),
                        (0.44, 0.48, 0.09),
                        (0.12, 0.44, 0.0),
                    ],
                    1.0,
                ),
            ],
        ),
        'F' => (
            0.80,
            vec![
                (
                    vec![(0.15, 1.03, 0.20), (0.08, 0.40, 0.18), (0.0, -0.10, 0.0)],
                    1.0,
                ),
                (vec![(0.10, 1.0, 0.17), (0.82, 1.06, 0.0)], 1.0),
                (vec![(0.09, 0.56, 0.15), (0.58, 0.60, 0.0)], 1.0),
            ],
        ),
        'r' => (
            0.58,
            vec![
                (vec![(0.12, 0.68, 0.17), (0.0, -0.08, 0.0)], 1.0),
                (
                    vec![(0.09, 0.40, 0.07), (0.22, 0.60, 0.12), (0.54, 0.70, 0.0)],
                    0.0,
                ),
            ],
        ),
        'e' => (
            0.68,
            vec![(
                vec![
                    (0.06, 0.32, 0.10),
                    (0.54, 0.38, 0.10),
                    (0.52, 0.56, 0.10),
                    (0.34, 0.68, 0.12),
                    (0.10, 0.58, 0.14),
                    (0.0, 0.30, 0.14),
                    (0.10, 0.04, 0.12),
                    (0.34, -0.02, 0.09),
                    (0.64, 0.12, 0.0),
                ],
                1.0,
            )],
        ),
        'O' => (0.90, vec![(ring(1.0), 1.0)]),
        '0' => (0.70, vec![(ring(0.75), 1.0)]),
        'S' => (
            0.80,
            vec![(
                vec![
                    (0.80, 1.06, 0.02),
                    (0.56, 1.02, 0.12),
                    (0.26, 1.0, 0.16),
                    (0.08, 0.86, 0.18),
                    (0.12, 0.66, 0.17),
                    (0.46, 0.50, 0.17),
                    (0.66, 0.34, 0.16),
                    (0.62, 0.10, 0.14),
                    (0.40, -0.02, 0.10),
                    (-0.04, 0.02, 0.0),
                ],
                0.0,
            )],
        ),
        'v' => (
            0.62,
            vec![
                (vec![(0.0, 0.68, 0.15), (0.27, -0.06, 0.0)], 1.0),
                (vec![(0.58, 0.72, 0.13), (0.25, -0.04, 0.0)], 1.0),
            ],
        ),
        '.' => (
            0.26,
            vec![(vec![(0.14, 0.16, 0.15), (0.04, -0.04, 0.0)], 1.0)],
        ),
        '1' => (
            0.44,
            vec![
                (vec![(0.32, 1.04, 0.17), (0.10, -0.10, 0.0)], 1.0),
                (vec![(0.31, 1.0, 0.11), (0.02, 0.78, 0.0)], 0.0),
            ],
        ),
        ' ' => (0.30, vec![]),
        _ => return None,
    })
}

/// True when every letter of `text` has a blade glyph.
pub fn covers(text: &str) -> bool {
    text.chars().all(|c| glyph(c).is_some())
}

/// How the letters are drawn.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    /// Forward lean, as x moved per unit of height.
    pub slant: f32,
    /// Extra space between letters, in cap heights.
    pub track: f32,
    /// How torn the thick ends are (1 = as drawn).
    pub torn: f32,
    /// Stroke weight (1 = as drawn).
    pub weight: f32,
    /// Picks the teeth and splinters; the same seed gives the same letters.
    pub seed: u32,
}

impl Default for Style {
    fn default() -> Style {
        Style {
            slant: 0.28,
            track: 0.10,
            torn: 1.0,
            weight: 1.0,
            seed: 7,
        }
    }
}

/// The outline polygons of one stroke (in glyph units).
fn stroke(pts: &[Pt], torn: f32, rng: &mut Rng, weight: f32) -> Vec<Vec<(f32, f32)>> {
    let n = pts.len();
    let (mut a, mut b) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for (i, &(x, y, w)) in pts.iter().enumerate() {
        let (dx, dy) = if i == 0 {
            (pts[1].0 - x, pts[1].1 - y)
        } else if i == n - 1 {
            (x - pts[i - 1].0, y - pts[i - 1].1)
        } else {
            (pts[i + 1].0 - pts[i - 1].0, pts[i + 1].1 - pts[i - 1].1)
        };
        let l = dx.hypot(dy).max(1e-6);
        let (nx, ny) = (-dy / l, dx / l);
        let w = w * weight;
        // The edges waver a little near the thick end.
        let j = (1.0 - i as f32 / (n - 1) as f32) * 0.03 * torn;
        let ea = w / 2.0 + (rng.f() - 0.5) * j;
        let eb = w / 2.0 + (rng.f() - 0.5) * j;
        a.push((x + nx * ea, y + ny * ea));
        b.push((x - nx * eb, y - ny * eb));
    }
    let (x0, y0, w0) = pts[0];
    let w0 = w0 * weight;
    let (dx, dy) = (pts[1].0 - x0, pts[1].1 - y0);
    let l = dx.hypot(dy).max(1e-6);
    let (ux, uy) = (dx / l, dy / l);
    let (nx, ny) = (-uy, ux);
    let mut polys = Vec::new();
    if torn > 0.0 {
        // Cut the start at an angle, then tear it into teeth.
        let cut = w0 * 0.6;
        a[0] = (a[0].0 - ux * cut, a[0].1 - uy * cut);
        let k = 3 + (rng.f() * 3.0) as usize;
        let mut poly = vec![b[0]];
        for j in 1..k {
            let t = j as f32 / k as f32;
            let (px, py) = (
                b[0].0 + (a[0].0 - b[0].0) * t,
                b[0].1 + (a[0].1 - b[0].1) * t,
            );
            let d = (0.3 + 0.7 * rng.f()) * w0 * 0.9 * torn * if j % 2 == 1 { 1.0 } else { -0.5 };
            poly.push((px - ux * d, py - uy * d));
        }
        poly.extend_from_slice(&a);
        poly.extend(b.iter().rev().take(n - 1));
        polys.push(poly);
        // Splinters: thin slivers flying off beside the torn end.
        for side in [1.0f32, -1.0] {
            if rng.f() < 0.6 * torn {
                let off = (w0 / 2.0 + 0.015) * side;
                let ln = 0.12 + 0.2 * rng.f();
                let ww = w0 * 0.25;
                let (sx, sy) = (x0 + nx * off - ux * 0.02, y0 + ny * off - uy * 0.02);
                let (mx, my) = (sx + ux * 0.03, sy + uy * 0.03);
                polys.push(vec![
                    (sx, sy),
                    (mx + nx * ww / 2.0, my + ny * ww / 2.0),
                    (sx + ux * ln, sy + uy * ln),
                    (mx - nx * ww / 2.0, my - ny * ww / 2.0),
                ]);
            }
        }
    } else {
        a.extend(b.iter().rev());
        polys.push(a);
    }
    polys
}

/// `text` in blade letters with caps `cap` pixels tall, cropped to its ink.
/// Letters without a glyph leave a gap.
pub fn render(text: &str, cap: f32, style: Style) -> Mask {
    let mut rng = Rng(0xB1AD_E000 ^ style.seed.wrapping_mul(0x9E37_79B9));
    let mut x = 0.15;
    let mut polys = Vec::new();
    for ch in text.chars() {
        let Some((adv, strokes)) = glyph(ch) else {
            x += 0.5 + style.track;
            continue;
        };
        for (pts, t) in strokes {
            let moved: Vec<Pt> = pts.iter().map(|&(a, b, w)| (x + a, b, w)).collect();
            polys.extend(stroke(&moved, t * style.torn, &mut rng, style.weight));
        }
        x += adv + style.track;
    }
    let wide = x + 0.4 + style.slant * 1.4;
    let (w, h) = ((wide * cap).ceil() as usize, (1.6 * cap).ceil() as usize);
    let px: Vec<Vec<(f32, f32)>> = polys
        .iter()
        .map(|p| {
            p.iter()
                .map(|&(gx, gy)| {
                    (
                        (gx + style.slant * (gy + 0.2) + 0.05) * cap,
                        (1.3 - gy) * cap,
                    )
                })
                .collect()
        })
        .collect();
    crop(&fill(&px, w, h))
}

/// Fill polygons (pixel coordinates, each even-odd, all unioned) into a
/// `w` x `h` mask, 4 x 4 samples per pixel.
pub fn fill(polys: &[Vec<(f32, f32)>], w: usize, h: usize) -> Mask {
    const SS: usize = 4;
    let mut m = Mask::new(w, h);
    let boxes: Vec<(f32, f32)> = polys
        .iter()
        .map(|p| {
            p.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &(_, y)| {
                (lo.min(y), hi.max(y))
            })
        })
        .collect();
    let sw = w * SS;
    let mut row = vec![false; sw];
    let mut count = vec![u16::default(); w];
    let mut xs: Vec<f32> = Vec::new();
    for y in 0..h {
        count.fill(0);
        for sy in 0..SS {
            row.fill(false);
            let fy = y as f32 + (sy as f32 + 0.5) / SS as f32;
            for (p, &(lo, hi)) in polys.iter().zip(&boxes) {
                if fy < lo || fy > hi {
                    continue;
                }
                xs.clear();
                for i in 0..p.len() {
                    let (a, b) = (p[i], p[(i + 1) % p.len()]);
                    if (a.1 <= fy) != (b.1 <= fy) {
                        xs.push(a.0 + (fy - a.1) / (b.1 - a.1) * (b.0 - a.0));
                    }
                }
                xs.sort_by(f32::total_cmp);
                for span in xs.chunks_exact(2) {
                    // Samples whose centres lie inside the span.
                    let s0 = ((span[0] * SS as f32 - 0.5).ceil().max(0.0)) as usize;
                    let s1 =
                        ((span[1] * SS as f32 - 0.5).floor() + 1.0).clamp(0.0, sw as f32) as usize;
                    if s0 < s1 {
                        row[s0..s1].fill(true);
                    }
                }
            }
            for (x, c) in count.iter_mut().enumerate() {
                *c += row[x * SS..(x + 1) * SS].iter().filter(|&&v| v).count() as u16;
            }
        }
        for (x, &c) in count.iter().enumerate() {
            m.a[y * w + x] = (c as u32 * 255 / (SS * SS) as u32) as u8;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_have_every_letter() {
        assert!(covers(&format!("{} OS", crate::TITLE)));
        assert!(covers(crate::VERSION));
    }

    #[test]
    fn same_seed_same_letters() {
        let a = render("MPFree OS", 40.0, Style::default());
        let b = render("MPFree OS", 40.0, Style::default());
        assert_eq!((a.w, a.h), (b.w, b.h));
        assert_eq!(a.a, b.a);
        // Wide, a line of leaning caps, and plenty of ink.
        assert!(a.w > a.h * 4, "{} x {}", a.w, a.h);
        let ink = a.a.iter().filter(|&&v| v > 128).count();
        assert!(ink > a.w * a.h / 8, "{ink}");
    }

    #[test]
    fn fill_covers_a_square() {
        let m = fill(
            &[vec![(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)]],
            8,
            8,
        );
        assert_eq!(m.at(3, 3), 255);
        assert_eq!(m.at(1, 1), 0);
        assert_eq!(m.at(7, 7), 0);
    }
}
