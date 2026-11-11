//! The title rasterised for the scene: each pixel's letter coverage plus a
//! dark outline around it, at a given cap height in pixels.
//!
//! The letters come from a [`TitleFont`]: the built-in block font
//! ([`crate::blockfont`], our own drawing) or any TrueType/OpenType file
//! loaded at run time (`--font PATH`). Changing the title's typeface is
//! therefore a matter of pointing at another font file; nothing else in the
//! scene depends on which font is used.

use std::path::Path;

use crate::blockfont;
use crate::galaxy::TITLE;

/// Where the title's letters come from.
#[derive(Clone, Default)]
pub enum TitleFont {
    /// Our own block letters (the default; no font file needed).
    #[default]
    Block,
    /// A TrueType/OpenType font. `cap` is its cap height as a fraction of
    /// the font size (the size fontdue is asked for); `tracking` is extra
    /// space between letters, in cap heights (see [`TitleFont::with_tracking`]).
    File {
        font: Box<fontdue::Font>,
        cap: f32,
        tracking: f32,
    },
}

impl TitleFont {
    /// Loads a .ttf or .otf file. Fails if it can't be parsed or lacks a
    /// letter of the title.
    pub fn load(path: &Path) -> Result<TitleFont, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        TitleFont::from_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<TitleFont, String> {
        let settings = fontdue::FontSettings {
            scale: 200.0,
            ..Default::default()
        };
        let font = fontdue::Font::from_bytes(bytes, settings)
            .map_err(|e| format!("not a usable font: {e}"))?;
        if let Some(c) = TITLE
            .chars()
            .find(|&c| c != ' ' && font.lookup_glyph_index(c) == 0)
        {
            return Err(format!("the font has no {c:?}"));
        }
        // Cap height: the tallest of a few flat-topped capitals, measured
        // at a large size so rounding doesn't matter.
        let probe = 1000.0;
        let cap = ['H', 'E', 'F', 'M']
            .iter()
            .filter(|&&c| font.lookup_glyph_index(c) != 0)
            .map(|&c| font.metrics(c, probe).height as f32 / probe)
            .fold(0.0f32, f32::max);
        if !(0.05..=2.0).contains(&cap) {
            return Err(format!("odd cap height ({cap})"));
        }
        Ok(TitleFont::File {
            font: Box::new(font),
            cap,
            tracking: 0.0,
        })
    }

    /// Extra space between letters, in cap heights (negative tightens).
    /// Some display fonts draw letters wider than their spacing allows,
    /// which hides small marks like the "." in "v0.1"; this opens them up.
    /// It has no effect on the block letters.
    pub fn with_tracking(mut self, cap_heights: f32) -> TitleFont {
        if let TitleFont::File { tracking, .. } = &mut self {
            *tracking = cap_heights.clamp(-1.0, 2.0);
        }
        self
    }

    pub fn name(&self) -> String {
        match self {
            TitleFont::Block => "built-in block letters".into(),
            TitleFont::File { font, .. } => font.name().unwrap_or("unnamed font").to_string(),
        }
    }
}

/// For each pixel, how much is letter (0..1) and how much is outline
/// (0..1, drawn under the letter), plus where the lines sit so a shader can
/// shade each line from its cap top to its baseline.
pub struct TitleMask {
    pub width: usize,
    pub height: usize,
    pub ink: Vec<f32>,
    pub halo: Vec<f32>,
    /// Pixels from the mask's top to the first line's cap top.
    pub cap_top: f32,
    /// Cap height in pixels.
    pub cap: f32,
    /// Pixels from one line's cap top to the next.
    pub pitch: f32,
}

/// The outline's width in pixels at this cap height.
fn outline(cap: f32) -> usize {
    ((cap / 14.0).round() as usize).max(1)
}

/// Line pitch as a multiple of the cap height (same for every font).
const PITCH: f32 = (blockfont::CAP + blockfont::LINE_GAP) / blockfont::CAP;

/// One glyph placed for a font file: its bitmap's top-left in pixels
/// relative to the first line's cap top and the centre line.
struct Placed {
    c: char,
    x: i32,
    y: i32,
    w: usize,
    h: usize,
}

/// Lays out `lines` for a font file at `cap` pixels: each line centred on
/// its own ink, lines `PITCH * cap` apart. Returns the glyphs and the ink's
/// bounds (left, top, right, bottom).
fn place(
    font: &fontdue::Font,
    font_cap: f32,
    tracking: f32,
    lines: &[&str],
    cap: f32,
) -> (Vec<Placed>, (i32, i32, i32, i32)) {
    let px = cap / font_cap;
    let mut out = Vec::new();
    let mut bounds = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (k, line) in lines.iter().enumerate() {
        let baseline = k as f32 * PITCH * cap + cap;
        let mut pen = 0.0f32;
        let mut prev: Option<char> = None;
        let start = out.len();
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for c in line.chars() {
            if let Some(p) = prev {
                pen += font.horizontal_kern(p, c, px).unwrap_or(0.0) + tracking * cap;
            }
            let m = font.metrics(c, px);
            if m.width > 0 && m.height > 0 {
                let x = (pen + m.xmin as f32).round() as i32;
                let y = (baseline - (m.ymin + m.height as i32) as f32).round() as i32;
                lo = lo.min(x);
                hi = hi.max(x + m.width as i32);
                out.push(Placed {
                    c,
                    x,
                    y,
                    w: m.width,
                    h: m.height,
                });
            }
            pen += m.advance_width;
            prev = Some(c);
        }
        if lo > hi {
            continue;
        }
        // Centre this line's ink on x = 0.
        let shift = -(lo + hi) / 2;
        for g in &mut out[start..] {
            g.x += shift;
            bounds.0 = bounds.0.min(g.x);
            bounds.1 = bounds.1.min(g.y);
            bounds.2 = bounds.2.max(g.x + g.w as i32);
            bounds.3 = bounds.3.max(g.y + g.h as i32);
        }
    }
    if out.is_empty() {
        bounds = (0, 0, 0, 0);
    }
    (out, bounds)
}

/// Size in pixels of `lines` at `cap` pixels per cap height, outline included.
pub fn size(font: &TitleFont, lines: &[&str], cap: f32) -> (usize, usize) {
    let pad = outline(cap) + 1;
    match font {
        TitleFont::Block => {
            let ppu = cap / blockfont::CAP;
            let (w, h) = blockfont::block_size(lines);
            (
                (w * ppu).ceil() as usize + 2 * pad,
                (h * ppu).ceil() as usize + 2 * pad,
            )
        }
        TitleFont::File {
            font,
            cap: fc,
            tracking,
        } => {
            let (_, (l, t, r, b)) = place(font, *fc, *tracking, lines, cap);
            ((r - l) as usize + 2 * pad, (b - t) as usize + 2 * pad)
        }
    }
}

/// Lays `lines` out centred, one above the other, at `cap` pixels per cap height.
pub fn rasterise(font: &TitleFont, lines: &[&str], cap: f32) -> TitleMask {
    let r = outline(cap);
    let pad = r + 1;
    let (w, h, ink, cap_top) = match font {
        TitleFont::Block => {
            let m = blockfont::rasterise(lines, cap / blockfont::CAP, pad);
            (m.width, m.height, m.coverage, pad as f32)
        }
        TitleFont::File {
            font,
            cap: fc,
            tracking,
        } => {
            let (glyphs, (l, t, rr, b)) = place(font, *fc, *tracking, lines, cap);
            let (w, h) = ((rr - l) as usize + 2 * pad, (b - t) as usize + 2 * pad);
            let mut ink = vec![0.0f32; w * h];
            let px = cap / fc;
            for g in &glyphs {
                let (m, bitmap) = font.rasterize(g.c, px);
                let (ox, oy) = (g.x - l + pad as i32, g.y - t + pad as i32);
                for gy in 0..m.height {
                    for gx in 0..m.width {
                        let (x, y) = (ox + gx as i32, oy + gy as i32);
                        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                            let o = &mut ink[y as usize * w + x as usize];
                            *o = (*o + bitmap[gy * m.width + gx] as f32 / 255.0).min(1.0);
                        }
                    }
                }
            }
            (w, h, ink, (pad as i32 - t) as f32)
        }
    };
    // The outline: the letters grown by r pixels (a round brush).
    let mut halo = vec![0.0f32; w * h];
    let ri = r as i64;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let mut best = 0.0f32;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    let (sx, sy) = (x + dx, y + dy);
                    if dx * dx + dy * dy <= ri * ri + ri
                        && sx >= 0
                        && sy >= 0
                        && (sx as usize) < w
                        && (sy as usize) < h
                    {
                        best = best.max(ink[sy as usize * w + sx as usize]);
                    }
                }
            }
            halo[y as usize * w + x as usize] = best;
        }
    }
    TitleMask {
        width: w,
        height: h,
        ink,
        halo,
        cap_top,
        cap,
        pitch: PITCH * cap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(font: &TitleFont) {
        for cap in [6.0, 12.0, 21.5, 40.0] {
            let m = rasterise(font, &["MPCFree", "OS v0.1"], cap);
            assert_eq!(
                (m.width, m.height),
                size(font, &["MPCFree", "OS v0.1"], cap),
                "{cap}"
            );
            assert!(m.halo.iter().zip(&m.ink).all(|(h, i)| h >= i));
            assert!(m.halo.iter().sum::<f32>() > m.ink.iter().sum::<f32>() * 1.1);
            assert!(m.cap_top >= 1.0 && m.cap_top < m.height as f32);
        }
    }

    #[test]
    fn block_letters() {
        check(&TitleFont::Block);
    }

    /// Any font on the build machine with the title's letters works the same
    /// way. (None is shipped in the repository; this is skipped without one.)
    #[test]
    fn a_system_font_file() {
        let candidates = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        ];
        let Some(path) = candidates.iter().map(Path::new).find(|p| p.exists()) else {
            eprintln!("no DejaVu font here; skipping");
            return;
        };
        let font = TitleFont::load(path).unwrap();
        let TitleFont::File { cap, .. } = &font else {
            unreachable!()
        };
        assert!(
            (0.6..0.8).contains(cap),
            "DejaVu's cap height is about 0.73 em, got {cap}"
        );
        check(&font);
        check(&font.clone().with_tracking(0.3));
        let wide = size(&font.clone().with_tracking(0.3), &["MPCFree"], 30.0).0;
        assert!(
            wide > size(&font, &["MPCFree"], 30.0).0 + 6 * 8,
            "tracking adds 0.3 cap between 7 letters"
        );
        // The first line's capitals start at cap_top: the row above is blank.
        let m = rasterise(&font, &["MPCFree"], 30.0);
        let row = |y: usize| m.ink[y * m.width..(y + 1) * m.width].iter().sum::<f32>();
        assert!(row(m.cap_top as usize + 1) > 1.0);
        // (Round letters overshoot the cap height a little: allow a faint edge.)
        assert!(row(m.cap_top as usize - 1) < 0.1 * row(m.cap_top as usize + 1));
    }

    #[test]
    fn not_a_font() {
        assert!(TitleFont::from_bytes(b"hello").is_err());
    }
}
