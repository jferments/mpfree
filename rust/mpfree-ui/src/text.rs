//! Fonts and text: three faces (sans, bold, mono), glyphs rasterised once per
//! size by fontdue and kept, drawn through `Canvas::blend_mask`.

use crate::canvas::Canvas;
use crate::color::Color;
use crate::geom::Rect;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub const INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular-Latin.ttf");
pub const INTER_SEMIBOLD: &[u8] = include_bytes!("../fonts/Inter-SemiBold-Latin.ttf");
pub const JETBRAINS_MONO: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular-Latin.ttf");

/// Glyphs kept per face before the cache is emptied (a safety net; the UI
/// uses a few sizes of a few hundred characters).
const MAX_CACHED: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontId {
    Sans,
    Bold,
    Mono,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

struct Glyph {
    xmin: i32,
    ymin: i32,
    width: usize,
    height: usize,
    advance: f32,
    bitmap: Vec<u8>,
}

pub struct Face {
    /// The parsed font, about 1.5 MB a face: shared by a face's copies.
    font: Arc<fontdue::Font>,
    cache: HashMap<(char, u32), Glyph>,
}

/// A copy shares the parsed font and starts its own glyph cache: a screen
/// built from a copy of the fonts costs no more memory for them (the GUI
/// smoke test keeps dozens of spare screens).
impl Clone for Face {
    fn clone(&self) -> Face {
        Face {
            font: Arc::clone(&self.font),
            cache: HashMap::new(),
        }
    }
}

fn key(px: f32) -> u32 {
    (px * 4.0).round() as u32
}

impl Face {
    pub fn from_bytes(bytes: &[u8]) -> Result<Face, String> {
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|e| e.to_string())?;
        Ok(Face {
            font: Arc::new(font),
            cache: HashMap::new(),
        })
    }

    pub fn from_file(path: &Path) -> Result<Face, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Face::from_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn glyph(&mut self, c: char, px: f32) -> &Glyph {
        if self.cache.len() >= MAX_CACHED {
            self.cache.clear();
        }
        let font = &self.font;
        self.cache.entry((c, key(px))).or_insert_with(|| {
            let (m, bitmap) = font.rasterize(c, px);
            Glyph {
                xmin: m.xmin,
                ymin: m.ymin,
                width: m.width,
                height: m.height,
                advance: m.advance_width,
                bitmap,
            }
        })
    }

    /// Distance from the baseline up to the top of a capital letter.
    pub fn cap_height(&mut self, px: f32) -> i32 {
        self.glyph('H', px).height as i32
    }

    pub fn ascent(&self, px: f32) -> f32 {
        self.font
            .horizontal_line_metrics(px)
            .map(|m| m.ascent)
            .unwrap_or(px * 0.8)
    }

    pub fn descent(&self, px: f32) -> f32 {
        self.font
            .horizontal_line_metrics(px)
            .map(|m| -m.descent)
            .unwrap_or(px * 0.2)
    }

    pub fn measure(&mut self, px: f32, text: &str) -> i32 {
        let w: f32 = text.chars().map(|c| self.glyph(c, px).advance).sum();
        w.round() as i32
    }

    /// Draw `text` with its baseline at `baseline`, starting at `x`. Returns
    /// the advance in pixels.
    pub fn draw(
        &mut self,
        canvas: &mut dyn Canvas,
        px: f32,
        x: i32,
        baseline: i32,
        color: Color,
        text: &str,
    ) -> i32 {
        let mut pen = x as f32;
        for c in text.chars() {
            let g = self.glyph(c, px);
            if g.width > 0 && g.height > 0 {
                let gx = pen.round() as i32 + g.xmin;
                let gy = baseline - g.height as i32 - g.ymin;
                canvas.blend_mask(gx, gy, g.width, g.height, &g.bitmap, color);
            }
            pen += g.advance;
        }
        (pen - x as f32).round() as i32
    }

    /// The longest start of `text` that fits in `width` pixels, with "…"
    /// added when something was cut.
    pub fn fit(&mut self, px: f32, text: &str, width: i32) -> String {
        if self.measure(px, text) <= width {
            return text.to_string();
        }
        let ell = self.glyph('…', px).advance;
        let mut used = 0.0;
        let mut out = String::new();
        for c in text.chars() {
            let a = self.glyph(c, px).advance;
            if used + a + ell > width as f32 {
                break;
            }
            used += a;
            out.push(c);
        }
        out.push('…');
        out
    }
}

/// The three faces. A clone shares their parsed fonts (see [`Face`]).
#[derive(Clone)]
pub struct Fonts {
    pub sans: Face,
    pub bold: Face,
    pub mono: Face,
    /// The face drawn for each role: a theme can swap them (a console theme
    /// draws everything in mono) without any widget changing.
    roles: [FontId; 3],
}

impl Fonts {
    /// The fonts built into the program (Inter and JetBrains Mono, OFL 1.1).
    pub fn embedded() -> Fonts {
        Fonts {
            sans: Face::from_bytes(INTER_REGULAR).expect("built-in Inter Regular"),
            bold: Face::from_bytes(INTER_SEMIBOLD).expect("built-in Inter SemiBold"),
            mono: Face::from_bytes(JETBRAINS_MONO).expect("built-in JetBrains Mono"),
            roles: [FontId::Sans, FontId::Bold, FontId::Mono],
        }
    }

    /// The built-in fonts, with any of the three replaced from a TTF/OTF file.
    pub fn with_files(
        sans: Option<&Path>,
        bold: Option<&Path>,
        mono: Option<&Path>,
    ) -> Result<Fonts, String> {
        let mut f = Fonts::embedded();
        if let Some(p) = sans {
            f.sans = Face::from_file(p)?;
        }
        if let Some(p) = bold {
            f.bold = Face::from_file(p)?;
        }
        if let Some(p) = mono {
            f.mono = Face::from_file(p)?;
        }
        Ok(f)
    }

    /// Draw `body` wherever a widget asks for [`FontId::Sans`] and `title`
    /// wherever it asks for [`FontId::Bold`] (the theme's `face_body` and
    /// `face_title`). Mono stays mono.
    pub fn set_roles(&mut self, body: FontId, title: FontId) {
        self.roles = [body, title, FontId::Mono];
    }

    /// The face drawn for `id`, after the theme's swaps.
    pub fn face(&mut self, id: FontId) -> &mut Face {
        let id = match id {
            FontId::Sans => self.roles[0],
            FontId::Bold => self.roles[1],
            FontId::Mono => self.roles[2],
        };
        match id {
            FontId::Sans => &mut self.sans,
            FontId::Bold => &mut self.bold,
            FontId::Mono => &mut self.mono,
        }
    }

    pub fn measure(&mut self, id: FontId, px: f32, text: &str) -> i32 {
        self.face(id).measure(px, text)
    }

    /// One line of text in `r`: centred vertically on its capital letters,
    /// aligned as asked, cut with "…" if it does not fit. Returns the width drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_in(
        &mut self,
        canvas: &mut dyn Canvas,
        id: FontId,
        px: f32,
        r: Rect,
        align: Align,
        color: Color,
        text: &str,
    ) -> i32 {
        let face = self.face(id);
        let shown = face.fit(px, text, r.w);
        let w = face.measure(px, &shown);
        let x = match align {
            Align::Left => r.x,
            Align::Center => r.x + (r.w - w) / 2,
            Align::Right => r.right() - w,
        };
        let cap = face.cap_height(px);
        let baseline = r.y + (r.h + cap) / 2;
        let old = canvas.clip();
        canvas.set_clip(old.intersect(r.inset_xy(0, -(px as i32))));
        face.draw(canvas, px, x, baseline, color, &shown);
        canvas.set_clip(old);
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::SoftCanvas;

    #[test]
    fn built_in_fonts_load_and_measure() {
        let mut f = Fonts::embedded();
        let w = f.measure(FontId::Sans, 20.0, "MPFree");
        assert!(w > 50 && w < 100, "{w}");
        // Mono: every character the same width.
        assert_eq!(
            f.measure(FontId::Mono, 14.0, "iiii"),
            f.measure(FontId::Mono, 14.0, "WWWW")
        );
        assert!(f.sans.cap_height(20.0) >= 13 && f.sans.cap_height(20.0) <= 16);
    }

    #[test]
    fn a_copy_of_the_fonts_shares_them_and_measures_the_same() {
        let mut f = Fonts::embedded();
        f.set_roles(FontId::Mono, FontId::Bold);
        let w = f.measure(FontId::Sans, 20.0, "MPFree");
        let mut copy = f.clone();
        assert!(Arc::ptr_eq(&f.sans.font, &copy.sans.font));
        assert!(copy.sans.cache.is_empty(), "a copy starts its own cache");
        assert_eq!(copy.measure(FontId::Sans, 20.0, "MPFree"), w);
        assert_eq!(
            copy.measure(FontId::Bold, 20.0, "Kit"),
            f.measure(FontId::Bold, 20.0, "Kit")
        );
    }

    #[test]
    fn long_text_is_cut_with_an_ellipsis() {
        let mut f = Fonts::embedded();
        let s = f
            .sans
            .fit(16.0, "a very long sample name that does not fit", 80);
        assert!(s.ends_with('…') && s.len() < 30, "{s}");
        assert!(f.sans.measure(16.0, &s) <= 80);
        assert_eq!(f.sans.fit(16.0, "Kick", 80), "Kick");
    }

    #[test]
    fn text_lands_inside_its_rectangle() {
        let mut f = Fonts::embedded();
        let mut c = SoftCanvas::new(200, 60);
        let r = Rect::new(20, 10, 160, 40);
        f.draw_in(
            &mut c,
            FontId::Bold,
            20.0,
            r,
            Align::Center,
            Color::rgb(0xFFFFFF),
            "Kit 1",
        );
        let mut lit = Rect::default();
        for y in 0..60 {
            for x in 0..200 {
                if c.pixel(x, y).r > 0 {
                    lit = lit.union(Rect::new(x, y, 1, 1));
                }
            }
        }
        assert!(!lit.is_empty());
        assert!(r.intersect(lit) == lit, "{lit:?} outside {r:?}");
        // Centred: about as much room left as right.
        assert!(
            ((lit.x - r.x) - (r.right() - lit.right())).abs() <= 3,
            "{lit:?}"
        );
    }
}
