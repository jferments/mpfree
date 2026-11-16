//! Pixels for skins: the fonts skins name, their images (decoded once and
//! kept within a memory budget), scaled and blended blits, and text placed
//! the way the MPC's toolkit (JUCE) places it.

use crate::model::{HAlign, RectF, TextStyle, VAlign};
use crate::png::{self, Picture};
use mpfree_ui::text::Face;
use mpfree_ui::{Canvas, Color, Rect, SoftCanvas};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const TITILLIUM_REGULAR: &[u8] = include_bytes!("../fonts/TitilliumWeb-Regular-Latin.ttf");
pub const TITILLIUM_SEMIBOLD: &[u8] = include_bytes!("../fonts/TitilliumWeb-SemiBold-Latin.ttf");
pub const TITILLIUM_BOLD: &[u8] = include_bytes!("../fonts/TitilliumWeb-Bold-Latin.ttf");
pub const ROBOTO_REGULAR: &[u8] = include_bytes!("../fonts/Roboto-Regular-Latin.ttf");
pub const ROBOTO_MEDIUM: &[u8] = include_bytes!("../fonts/Roboto-Medium-Latin.ttf");
pub const ROBOTO_SEMIBOLD: &[u8] = include_bytes!("../fonts/Roboto-SemiBold-Latin.ttf");
pub const ROBOTO_BOLD: &[u8] = include_bytes!("../fonts/Roboto-Bold-Latin.ttf");

/// The fonts a skin may name, parsed on first use. Titillium Web is the
/// MPC's own UI font and stands in for any name it doesn't know (Nexa
/// included, which MPFree can't ship).
#[derive(Default)]
pub struct Fonts {
    faces: HashMap<&'static str, (Face, f32, f32)>,
}

impl Fonts {
    fn file(name: &str, style: &str) -> (&'static str, &'static [u8]) {
        let s = style.to_ascii_lowercase();
        let heavy = s.contains("bold") || s.contains("black") || s.contains("heavy");
        let semi = s.contains("semibold") || s.contains("demi");
        let medium = s.contains("medium");
        if name.eq_ignore_ascii_case("Roboto") {
            if semi {
                ("roboto-semibold", ROBOTO_SEMIBOLD)
            } else if heavy {
                ("roboto-bold", ROBOTO_BOLD)
            } else if medium {
                ("roboto-medium", ROBOTO_MEDIUM)
            } else {
                ("roboto-regular", ROBOTO_REGULAR)
            }
        } else if semi {
            ("titillium-semibold", TITILLIUM_SEMIBOLD)
        } else if heavy {
            ("titillium-bold", TITILLIUM_BOLD)
        } else {
            ("titillium-regular", TITILLIUM_REGULAR)
        }
    }

    /// The face for a skin's font, the font size in pixels for a JUCE
    /// height of `height` (ascent plus descent), and the ascent at that size.
    pub fn face(&mut self, name: &str, style: &str, height: f32) -> (&mut Face, f32, f32) {
        let (key, bytes) = Fonts::file(name, style);
        let (face, em_per_height, ascent_share) = self.faces.entry(key).or_insert_with(|| {
            let face = Face::from_bytes(bytes).expect("built-in font");
            let (a, d) = (face.ascent(100.0), face.descent(100.0));
            let total = (a + d).max(1.0);
            (face, 100.0 / total, a / total)
        });
        let px = height * *em_per_height;
        let ascent = height * *ascent_share;
        (face, px, ascent)
    }
}

/// How much decoded image memory a skin may keep (the MPC One has 2 GB;
/// the biggest catalog skin needs about 270 MB for every picture at once,
/// and one tab far less).
pub const IMAGE_BUDGET: usize = 160 << 20;

struct Slot {
    pic: Option<Arc<Picture>>,
    used: u64,
}

/// Decoded images by path, kept within [`IMAGE_BUDGET`]: the least recently
/// drawn go first.
pub struct Images {
    /// The plugin folder: no image is read from outside it.
    root: PathBuf,
    cache: HashMap<PathBuf, Slot>,
    bytes: usize,
    tick: u64,
    pub budget: usize,
    /// Images that failed to load, with why (for the log).
    pub failures: Vec<String>,
}

impl Images {
    pub fn new(root: &Path) -> Images {
        Images {
            root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
            cache: HashMap::new(),
            bytes: 0,
            tick: 0,
            budget: IMAGE_BUDGET,
            failures: Vec::new(),
        }
    }

    /// Marks the start of a frame (for the least-recently-drawn order).
    pub fn frame(&mut self) {
        self.tick += 1;
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The picture at `rel` (relative to `dir`), decoded on first use.
    /// `None` for an empty path, a missing or broken file, or a path
    /// outside the plugin folder: the MPC draws nothing then.
    pub fn get(&mut self, dir: &Path, rel: &str) -> Option<Arc<Picture>> {
        if rel.is_empty() {
            return None;
        }
        let path = dir.join(rel);
        let tick = self.tick;
        if let Some(s) = self.cache.get_mut(&path) {
            s.used = tick;
            return s.pic.clone();
        }
        let pic = self.load(&path);
        let size = pic.as_ref().map_or(0, |p| p.bytes());
        self.make_room(size);
        self.bytes += size;
        self.cache.insert(
            path,
            Slot {
                pic: pic.clone(),
                used: tick,
            },
        );
        pic
    }

    fn load(&mut self, path: &Path) -> Option<Arc<Picture>> {
        let real = path.canonicalize().ok()?;
        if !real.starts_with(&self.root) {
            self.failures
                .push(format!("{}: outside the plugin folder", path.display()));
            return None;
        }
        let bytes = std::fs::read(&real).ok()?;
        match png::decode(&bytes) {
            Ok(p) => Some(Arc::new(p)),
            Err(e) => {
                self.failures.push(format!("{}: {e}", path.display()));
                None
            }
        }
    }

    fn make_room(&mut self, need: usize) {
        while self.bytes + need > self.budget && !self.cache.is_empty() {
            let oldest = self
                .cache
                .iter()
                .filter(|(_, s)| s.used < self.tick)
                .min_by_key(|(_, s)| s.used)
                .map(|(p, _)| p.clone());
            let Some(p) = oldest else { break };
            if let Some(s) = self.cache.remove(&p) {
                self.bytes -= s.pic.map_or(0, |p| p.bytes());
            }
        }
    }
}

/// A fractional rectangle to whole pixels.
pub fn px(r: RectF) -> Rect {
    let x0 = r.x.round() as i32;
    let y0 = r.y.round() as i32;
    let x1 = (r.x + r.w).round() as i32;
    let y1 = (r.y + r.h).round() as i32;
    Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
}

/// Straight ARGB to a colour.
pub fn color(argb: u32) -> Color {
    Color {
        r: (argb >> 16) as u8,
        g: (argb >> 8) as u8,
        b: argb as u8,
        a: (argb >> 24) as u8,
    }
}

pub fn fill(c: &mut SoftCanvas, r: RectF, argb: u32) {
    if argb >> 24 != 0 {
        c.fill_rect(px(r), color(argb));
    }
}

/// An outline `t` pixels thick inside `r`.
pub fn outline(c: &mut SoftCanvas, r: RectF, t: f32, argb: u32) {
    if argb >> 24 == 0 || t <= 0.0 {
        return;
    }
    let r = px(r);
    let t = (t.round() as i32).max(1);
    let col = color(argb);
    c.fill_rect(Rect::new(r.x, r.y, r.w, t), col);
    c.fill_rect(Rect::new(r.x, r.bottom() - t, r.w, t), col);
    c.fill_rect(Rect::new(r.x, r.y + t, t, r.h - 2 * t), col);
    c.fill_rect(Rect::new(r.right() - t, r.y + t, t, r.h - 2 * t), col);
}

#[inline]
fn over(dst: u32, src: u32) -> u32 {
    let a = src >> 24;
    if a == 255 {
        return src & 0xFF_FFFF;
    }
    if a == 0 {
        return dst;
    }
    let inv = 255 - a;
    let ch = |s: u32| {
        let d = (dst >> s) & 0xFF;
        let v = ((src >> s) & 0xFF) + (d * inv + 127) / 255;
        v.min(255) << s
    };
    ch(16) | ch(8) | ch(0)
}

#[inline]
fn lerp_px(a: u32, b: u32, t: u32) -> u32 {
    // t in 0..=256
    let mut out = 0;
    for s in [0, 8, 16, 24] {
        let x = (a >> s) & 0xFF;
        let y = (b >> s) & 0xFF;
        out |= ((x * (256 - t) + y * t) >> 8) << s;
    }
    out
}

/// Draws the part `src` (x, y, w, h in the picture) of `pic` scaled into
/// `dst`, blended over what is there, cut to the canvas's clip and `clip`.
/// `opacity` 0 to 255.
pub fn blit(
    c: &mut SoftCanvas,
    pic: &Picture,
    src: (u32, u32, u32, u32),
    dst: RectF,
    clip: Rect,
    opacity: u32,
) {
    let (sx, sy, sw, sh) = src;
    if sw == 0 || sh == 0 || dst.w <= 0.0 || dst.h <= 0.0 || opacity == 0 {
        return;
    }
    let area = px(dst).intersect(c.clip()).intersect(clip);
    if area.is_empty() {
        return;
    }
    let (cw, _) = c.size();
    let kx = sw as f32 / dst.w;
    let ky = sh as f32 / dst.h;
    let fade = |p: u32| {
        if opacity >= 255 {
            p
        } else {
            lerp_px(0, p, opacity + 1)
        }
    };
    let exact = (kx - 1.0).abs() < 1e-4
        && (ky - 1.0).abs() < 1e-4
        && (dst.x - dst.x.round()).abs() < 1e-3
        && (dst.y - dst.y.round()).abs() < 1e-3;
    if exact {
        let ox = dst.x.round() as i32;
        let oy = dst.y.round() as i32;
        for y in area.y..area.bottom() {
            let py = sy as i32 + (y - oy);
            if py < 0 || py >= (sy + sh) as i32 || py >= pic.h as i32 {
                continue;
            }
            let srow = (py as u32 * pic.w) as usize;
            let drow = (y * cw) as usize;
            for x in area.x..area.right() {
                let pxx = sx as i32 + (x - ox);
                if pxx < 0 || pxx >= (sx + sw) as i32 || pxx >= pic.w as i32 {
                    continue;
                }
                let s = fade(pic.px[srow + pxx as usize]);
                let d = &mut c.px[drow + x as usize];
                *d = over(*d, s);
            }
        }
        return;
    }
    let sample = |fx: f32, fy: f32| -> u32 {
        // Bilinear, clamped to the source part.
        let fx = (fx - 0.5).clamp(0.0, sw as f32 - 1.0);
        let fy = (fy - 0.5).clamp(0.0, sh as f32 - 1.0);
        let x0 = fx.floor() as u32;
        let y0 = fy.floor() as u32;
        let x1 = (x0 + 1).min(sw - 1);
        let y1 = (y0 + 1).min(sh - 1);
        let tx = ((fx - x0 as f32) * 256.0) as u32;
        let ty = ((fy - y0 as f32) * 256.0) as u32;
        let at = |x: u32, y: u32| {
            let (x, y) = ((sx + x).min(pic.w - 1), (sy + y).min(pic.h - 1));
            pic.px[(y * pic.w + x) as usize]
        };
        let top = lerp_px(at(x0, y0), at(x1, y0), tx);
        let bot = lerp_px(at(x0, y1), at(x1, y1), tx);
        lerp_px(top, bot, ty)
    };
    let box_avg = |fx0: f32, fy0: f32, fx1: f32, fy1: f32| -> u32 {
        let x0 = (fx0.floor().max(0.0) as u32).min(sw - 1);
        let y0 = (fy0.floor().max(0.0) as u32).min(sh - 1);
        let x1 = (fx1.ceil() as u32).clamp(x0 + 1, sw);
        let y1 = (fy1.ceil() as u32).clamp(y0 + 1, sh);
        let mut acc = [0u32; 4];
        let mut n = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let p =
                    pic.px[((sy + y).min(pic.h - 1) * pic.w + (sx + x).min(pic.w - 1)) as usize];
                for (k, a) in acc.iter_mut().enumerate() {
                    *a += (p >> (k * 8)) & 0xFF;
                }
                n += 1;
            }
        }
        let n = n.max(1);
        acc.iter()
            .enumerate()
            .fold(0, |o, (k, a)| o | ((a / n) << (k * 8)))
    };
    let shrink = kx > 2.0 || ky > 2.0;
    for y in area.y..area.bottom() {
        let fy = (y as f32 + 0.5 - dst.y) * ky;
        let drow = (y * cw) as usize;
        for x in area.x..area.right() {
            let fx = (x as f32 + 0.5 - dst.x) * kx;
            let s = if shrink {
                box_avg(fx - kx / 2.0, fy - ky / 2.0, fx + kx / 2.0, fy + ky / 2.0)
            } else {
                sample(fx, fy)
            };
            let d = &mut c.px[drow + x as usize];
            *d = over(*d, fade(s));
        }
    }
}

/// Draws `pic` whole, scaled to fill `dst`.
pub fn picture(c: &mut SoftCanvas, pic: &Picture, dst: RectF, clip: Rect) {
    blit(c, pic, (0, 0, pic.w, pic.h), dst, clip, 255);
}

/// Frame `frame` of a film strip (frames stacked vertically) scaled into
/// `dst`.
pub fn frame(c: &mut SoftCanvas, pic: &Picture, frames: u32, frame: u32, dst: RectF, clip: Rect) {
    let frames = frames.max(1);
    let fh = pic.h / frames;
    if fh == 0 {
        return;
    }
    let f = frame.min(frames - 1);
    blit(c, pic, (0, f * fh, pic.w, fh), dst, clip, 255);
}

/// One line of text in `r`, placed as JUCE places it (the font's ascent
/// plus descent is its height), cut with "…" when too long.
pub fn text(
    c: &mut SoftCanvas,
    fonts: &mut Fonts,
    style: &TextStyle,
    r: RectF,
    s: &str,
    clip: Rect,
) {
    if s.is_empty() || style.colour >> 24 == 0 {
        return;
    }
    let s = style.apply_case(s);
    let (face, size, ascent) = fonts.face(&style.font, &style.style, style.height);
    let width = r.w.max(0.0) as i32;
    let shown = face.fit(size, &s, width);
    let w = face.measure(size, &shown) as f32;
    let x = match style.h_align {
        HAlign::Left => r.x,
        HAlign::Centre => r.x + (r.w - w) / 2.0,
        HAlign::Right => r.x + r.w - w,
    };
    let top = match style.v_align {
        VAlign::Top => r.y,
        VAlign::Centre => r.y + (r.h - style.height) / 2.0,
        VAlign::Bottom => r.y + r.h - style.height,
    };
    let old = c.clip();
    c.set_clip(old.intersect(clip).intersect(px(r)));
    face.draw(
        c,
        size,
        x.round() as i32,
        (top + ascent).round() as i32,
        color(style.colour),
        &shown,
    );
    c.set_clip(old);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blits_blend_and_scale() {
        let mut c = SoftCanvas::new(8, 8);
        c.fill_rect(Rect::new(0, 0, 8, 8), Color::rgb(0x000000));
        // A 2x2 picture: opaque red, half white, clear, opaque blue.
        let pic = Picture {
            w: 2,
            h: 2,
            px: vec![
                0xFFFF_0000,
                png::premultiply(255, 255, 255, 128),
                0,
                0xFF00_00FF,
            ],
        };
        picture(
            &mut c,
            &pic,
            RectF::new(1.0, 1.0, 2.0, 2.0),
            Rect::new(0, 0, 8, 8),
        );
        assert_eq!(c.pixel(1, 1), Color::rgb(0xFF0000));
        assert_eq!(c.pixel(2, 1), Color::rgb(0x808080));
        assert_eq!(c.pixel(1, 2), Color::rgb(0x000000));
        assert_eq!(c.pixel(2, 2), Color::rgb(0x0000FF));
        // Scaled up 2x: the corners keep their colours.
        picture(
            &mut c,
            &pic,
            RectF::new(4.0, 4.0, 4.0, 4.0),
            Rect::new(0, 0, 8, 8),
        );
        assert_eq!(c.pixel(4, 4), Color::rgb(0xFF0000));
        assert_eq!(c.pixel(7, 7), Color::rgb(0x0000FF));
        // The clip holds.
        let mut c2 = SoftCanvas::new(4, 4);
        picture(
            &mut c2,
            &pic,
            RectF::new(0.0, 0.0, 2.0, 2.0),
            Rect::new(1, 1, 3, 3),
        );
        assert_eq!(c2.pixel(0, 0), Color::rgb(0));
    }

    #[test]
    fn film_strip_frames() {
        // Three 1x1 frames: red, green, blue.
        let pic = Picture {
            w: 1,
            h: 3,
            px: vec![0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF],
        };
        let mut c = SoftCanvas::new(1, 1);
        frame(
            &mut c,
            &pic,
            3,
            1,
            RectF::new(0.0, 0.0, 1.0, 1.0),
            Rect::new(0, 0, 1, 1),
        );
        assert_eq!(c.pixel(0, 0), Color::rgb(0x00FF00));
        frame(
            &mut c,
            &pic,
            3,
            9,
            RectF::new(0.0, 0.0, 1.0, 1.0),
            Rect::new(0, 0, 1, 1),
        );
        assert_eq!(c.pixel(0, 0), Color::rgb(0x0000FF));
    }

    #[test]
    fn fonts_resolve_and_size_like_juce() {
        let mut f = Fonts::default();
        let (_, px, ascent) = f.face("Titillium Web", "SemiBold", 30.0);
        // Titillium Web: ascent 1133, descent 388 per 1000 units.
        assert!((px - 30.0 * 1000.0 / 1521.0).abs() < 0.3, "{px}");
        assert!((ascent - 30.0 * 1133.0 / 1521.0).abs() < 0.3, "{ascent}");
        let (_, px, _) = f.face("Roboto", "Medium", 24.0);
        assert!((px - 24.0 * 2048.0 / 2400.0).abs() < 0.3, "{px}");
        // Unknown names fall back to Titillium Web.
        let (_, px, _) = f.face("Nexa", "Bold", 30.0);
        assert!((px - 30.0 * 1000.0 / 1521.0).abs() < 0.3, "{px}");
    }
}
