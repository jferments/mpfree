//! The title rasterised for the character-cell scene: the block font's
//! coverage plus a dark outline around it, at a given cap height in pixels.

use crate::blockfont;

/// For each pixel, how much is letter (0..1) and how much is outline
/// (0..1, drawn under the letter).
pub struct TitleMask {
    pub width: usize,
    pub height: usize,
    pub ink: Vec<f32>,
    pub halo: Vec<f32>,
}

/// The outline's width in pixels at this cap height.
fn outline(cap: f32) -> usize {
    ((cap / 14.0).round() as usize).max(1)
}

/// Size in pixels of `lines` at `cap` pixels per cap height, outline included.
pub fn size(lines: &[&str], cap: f32) -> (usize, usize) {
    let ppu = cap / blockfont::CAP;
    let (w, h) = blockfont::block_size(lines);
    let pad = outline(cap) + 1;
    ((w * ppu).ceil() as usize + 2 * pad, (h * ppu).ceil() as usize + 2 * pad)
}

/// Lays `lines` out centred, one above the other, at `cap` pixels per cap height.
pub fn rasterise(lines: &[&str], cap: f32) -> TitleMask {
    let r = outline(cap);
    let m = blockfont::rasterise(lines, cap / blockfont::CAP, r + 1);
    let (w, h) = (m.width, m.height);
    // The outline: the letters grown by r pixels (a round brush).
    let mut halo = vec![0.0f32; w * h];
    let ri = r as i64;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let mut best = 0.0f32;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    let (sx, sy) = (x + dx, y + dy);
                    if dx * dx + dy * dy <= ri * ri + ri && sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                        best = best.max(m.coverage[sy as usize * w + sx as usize]);
                    }
                }
            }
            halo[y as usize * w + x as usize] = best;
        }
    }
    TitleMask { width: w, height: h, ink: m.coverage, halo }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_matches_the_raster_and_the_outline_surrounds_the_ink() {
        for cap in [6.0, 12.0, 21.5, 40.0] {
            let m = rasterise(&["MPCFree", "OS v0.1"], cap);
            assert_eq!((m.width, m.height), size(&["MPCFree", "OS v0.1"], cap), "{cap}");
            assert!(m.halo.iter().zip(&m.ink).all(|(h, i)| h >= i));
            assert!(m.halo.iter().sum::<f32>() > m.ink.iter().sum::<f32>() * 1.1);
        }
    }
}
