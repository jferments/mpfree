//! Small pixel tools shared by the scenes: packed XRGB colours, additive
//! light, text masks, blur and a repeatable random generator.

use fontdue::{Font, FontSettings};

#[inline]
pub fn rgb(r: u32, g: u32, b: u32) -> u32 {
    (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
}

#[inline]
pub const fn hex(h: u32) -> [f32; 3] {
    [((h >> 16) & 255) as f32, ((h >> 8) & 255) as f32, (h & 255) as f32]
}

#[inline]
pub fn pack(c: [f32; 3]) -> u32 {
    let q = |v: f32| v.clamp(0.0, 255.0) as u32;
    (q(c[0]) << 16) | (q(c[1]) << 8) | q(c[2])
}

#[inline]
pub fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// `p` plus light `c` scaled by `k` (0-256), each channel saturating.
#[inline]
pub fn add(p: u32, c: u32, k: u32) -> u32 {
    let ch = |s: u32| (((p >> s) & 255) + ((((c >> s) & 255) * k) >> 8)).min(255) << s;
    ch(16) | ch(8) | ch(0)
}

/// `p` towards `c` by `a` (0-256).
#[inline]
pub fn over(p: u32, c: u32, a: u32) -> u32 {
    if a >= 256 {
        return c;
    }
    let inv = 256 - a;
    let ch = |s: u32| ((((p >> s) & 255) * inv + ((c >> s) & 255) * a) >> 8) << s;
    ch(16) | ch(8) | ch(0)
}

/// `p` darkened to `k`/256.
#[inline]
pub fn scale(p: u32, k: u32) -> u32 {
    let ch = |s: u32| ((((p >> s) & 255) * k) >> 8) << s;
    ch(16) | ch(8) | ch(0)
}

/// An 8-bit coverage mask.
#[derive(Clone, Debug, Default)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub a: Vec<u8>,
}

impl Mask {
    pub fn new(w: usize, h: usize) -> Mask {
        Mask { w, h, a: vec![0; w * h] }
    }

    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.a[y * self.w + x]
    }

    /// A copy grown by `pad` on every side.
    pub fn padded(&self, pad: usize) -> Mask {
        let mut m = Mask::new(self.w + 2 * pad, self.h + 2 * pad);
        for y in 0..self.h {
            let d = (y + pad) * m.w + pad;
            m.a[d..d + self.w].copy_from_slice(&self.a[y * self.w..(y + 1) * self.w]);
        }
        m
    }

    /// Thicker: each pixel takes the largest value within `r` (a square).
    pub fn dilated(&self, r: usize) -> Mask {
        let (w, h) = (self.w, self.h);
        let mut tmp = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let (lo, hi) = (x.saturating_sub(r), (x + r).min(w - 1));
                tmp[y * w + x] = *self.a[y * w + lo..=y * w + hi].iter().max().unwrap();
            }
        }
        let mut out = Mask::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let (lo, hi) = (y.saturating_sub(r), (y + r).min(h - 1));
                out.a[y * w + x] = (lo..=hi).map(|k| tmp[k * w + x]).max().unwrap();
            }
        }
        out
    }

    /// A box blur of radius `r`, run `passes` times (3 passes look Gaussian).
    pub fn blurred(&self, r: usize, passes: usize) -> Mask {
        let mut v: Vec<u32> = self.a.iter().map(|&x| x as u32).collect();
        let (w, h) = (self.w, self.h);
        let mut tmp = vec![0u32; w * h];
        let n = (2 * r + 1) as u32;
        for _ in 0..passes {
            for y in 0..h {
                let row = &v[y * w..(y + 1) * w];
                let mut s: u32 = row[..r.min(w)].iter().sum();
                for x in 0..w {
                    if x + r < w {
                        s += row[x + r];
                    }
                    tmp[y * w + x] = s / n;
                    if x >= r {
                        s -= row[x - r];
                    }
                }
            }
            for x in 0..w {
                let mut s: u32 = (0..r.min(h)).map(|y| tmp[y * w + x]).sum();
                for y in 0..h {
                    if y + r < h {
                        s += tmp[(y + r) * w + x];
                    }
                    v[y * w + x] = s / n;
                    if y >= r {
                        s -= tmp[(y - r) * w + x];
                    }
                }
            }
        }
        Mask { w, h, a: v.into_iter().map(|x| x.min(255) as u8).collect() }
    }
}

pub fn font_sans() -> Font {
    Font::from_bytes(mpfree_ui::text::INTER_SEMIBOLD, FontSettings::default()).expect("built-in Inter SemiBold")
}

pub fn font_regular() -> Font {
    Font::from_bytes(mpfree_ui::text::INTER_REGULAR, FontSettings::default()).expect("built-in Inter Regular")
}

pub fn font_mono() -> Font {
    Font::from_bytes(mpfree_ui::text::JETBRAINS_MONO, FontSettings::default()).expect("built-in JetBrains Mono")
}

/// `text` as a mask at `px`, with `tracking` extra pixels between letters.
/// The mask is cropped to the ink; returns it and the cap height.
pub fn text_mask(font: &Font, px: f32, tracking: f32, text: &str) -> Mask {
    let lm = font.horizontal_line_metrics(px).expect("horizontal font");
    let asc = lm.ascent.ceil() as i32;
    let height = (lm.ascent - lm.descent).ceil() as i32 + 2;
    let mut glyphs = Vec::new();
    let mut pen = 0.0f32;
    for c in text.chars() {
        let (m, bmp) = font.rasterize(c, px);
        glyphs.push((pen.round() as i32 + m.xmin, asc - m.height as i32 - m.ymin, m.width, m.height, bmp));
        pen += m.advance_width + tracking;
    }
    let width = (pen - tracking).ceil() as i32 + 2;
    let mut out = Mask::new(width.max(1) as usize, height.max(1) as usize);
    for (gx, gy, w, h, bmp) in glyphs {
        for y in 0..h {
            for x in 0..w {
                let (ox, oy) = (gx + x as i32, gy + y as i32);
                if ox >= 0 && oy >= 0 && (ox as usize) < out.w && (oy as usize) < out.h {
                    let i = oy as usize * out.w + ox as usize;
                    out.a[i] = out.a[i].max(bmp[y * w + x]);
                }
            }
        }
    }
    crop(&out)
}

/// The mask cut down to its ink.
pub fn crop(m: &Mask) -> Mask {
    let (mut x0, mut y0, mut x1, mut y1) = (m.w, m.h, 0, 0);
    for y in 0..m.h {
        for x in 0..m.w {
            if m.at(x, y) > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 {
        return Mask::new(1, 1);
    }
    let mut out = Mask::new(x1 - x0, y1 - y0);
    for y in y0..y1 {
        out.a[(y - y0) * out.w..(y - y0 + 1) * out.w].copy_from_slice(&m.a[y * m.w + x0..y * m.w + x1]);
    }
    out
}

/// Draw `m` in colour `c` at (x, y) into a `w`-wide pixel buffer, alpha `k`
/// (0-256), clipped to the buffer.
pub fn blit_mask(px: &mut [u32], w: usize, x: i32, y: i32, m: &Mask, c: u32, k: u32) {
    let h = px.len() / w;
    for my in 0..m.h {
        let py = y + my as i32;
        if py < 0 || py as usize >= h {
            continue;
        }
        for mx in 0..m.w {
            let a = m.a[my * m.w + mx] as u32;
            if a == 0 {
                continue;
            }
            let pxx = x + mx as i32;
            if pxx < 0 || pxx as usize >= w {
                continue;
            }
            let i = py as usize * w + pxx as usize;
            px[i] = over(px[i], c, (a * k) >> 8);
        }
    }
}

/// Add `m` as light in colour `c`, strength `k` (0-256).
pub fn glow_mask(px: &mut [u32], w: usize, x: i32, y: i32, m: &Mask, c: u32, k: u32) {
    let h = px.len() / w;
    for my in 0..m.h {
        let py = y + my as i32;
        if py < 0 || py as usize >= h {
            continue;
        }
        for mx in 0..m.w {
            let a = m.a[my * m.w + mx] as u32;
            if a == 0 {
                continue;
            }
            let pxx = x + mx as i32;
            if pxx < 0 || pxx as usize >= w {
                continue;
            }
            let i = py as usize * w + pxx as usize;
            px[i] = add(px[i], c, (a * k) >> 8);
        }
    }
}

/// A repeatable xorshift generator, so every run draws the same frames.
pub struct Rng(pub u32);

impl Rng {
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// 0.0 to 1.0.
    pub fn f(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n.max(1)
    }
}

/// A hash of two integers to 0..2^32, for noise that depends only on place
/// and time step.
#[inline]
pub fn hash(a: u32, b: u32) -> u32 {
    let mut x = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^ (x >> 15)
}

pub fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A line of light from (x0, y0) to (x1, y1), `c` at strength `k` (0-256),
/// fading from `k0` at the start to `k` at the end. One pixel wide.
pub fn light_line(px: &mut [u32], w: usize, (x0, y0): (f32, f32), (x1, y1): (f32, f32), c: u32, k0: u32, k: u32) {
    let h = px.len() / w;
    let n = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
    for i in 0..=n {
        let f = i as f32 / n as f32;
        let x = (x0 + (x1 - x0) * f) as i32;
        let y = (y0 + (y1 - y0) * f) as i32;
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            let kk = k0 as f32 + (k as f32 - k0 as f32) * f;
            let j = y as usize * w + x as usize;
            px[j] = add(px[j], c, kk as u32);
        }
    }
}

/// A layer of colour with coverage, drawn over a frame.
pub struct Sprite {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    /// (offset in the sprite, colour, alpha 0-256) for every covered pixel.
    pub px: Vec<(u32, u32, u16)>,
}

impl Sprite {
    pub fn from(x: i32, y: i32, m: &Mask, color: impl Fn(usize, usize) -> u32) -> Sprite {
        let mut px = Vec::new();
        for yy in 0..m.h {
            for xx in 0..m.w {
                let a = m.at(xx, yy) as u32;
                if a > 0 {
                    px.push(((yy * m.w + xx) as u32, color(xx, yy), (a + 1) as u16));
                }
            }
        }
        Sprite { x, y, w: m.w, px }
    }

    pub fn draw(&self, out: &mut [u32], w: usize, k: u32, light: bool) {
        self.draw_upto(out, w, k, light, i32::MAX)
    }

    /// Draw as light with its top left at (x, y) instead of its own place.
    pub fn draw_at(&self, out: &mut [u32], w: usize, x: i32, y: i32, k: u32) {
        let h = (out.len() / w) as i32;
        for &(o, c, a) in &self.px {
            let (sx, sy) = ((o as usize % self.w) as i32 + x, (o as usize / self.w) as i32 + y);
            if sx >= 0 && sy >= 0 && (sx as usize) < w && sy < h {
                let i = sy as usize * w + sx as usize;
                out[i] = add(out[i], c, (a as u32 * k) >> 8);
            }
        }
    }

    /// Its height in pixels (to its last covered row).
    pub fn h(&self) -> usize {
        self.px.last().map(|p| p.0 as usize / self.w.max(1) + 1).unwrap_or(0)
    }

    /// Draw as light at (x, y), only the columns left of `upto`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_upto_at(&self, out: &mut [u32], w: usize, x: i32, y: i32, k: u32, upto: i32) {
        let h = (out.len() / w) as i32;
        for &(o, c, a) in &self.px {
            let ox = (o as usize % self.w) as i32;
            if ox >= upto {
                continue;
            }
            let (sx, sy) = (ox + x, (o as usize / self.w) as i32 + y);
            if sx >= 0 && sy >= 0 && (sx as usize) < w && sy < h {
                let i = sy as usize * w + sx as usize;
                out[i] = add(out[i], c, (a as u32 * k) >> 8);
            }
        }
    }

    /// Draw only the columns left of `upto` (in the sprite), for wipes and typing.
    pub fn draw_upto(&self, out: &mut [u32], w: usize, k: u32, light: bool, upto: i32) {
        let h = out.len() / w;
        for &(o, c, a) in &self.px {
            let ox = (o as usize % self.w) as i32;
            if ox >= upto {
                continue;
            }
            let (sx, sy) = (ox + self.x, (o as usize / self.w) as i32 + self.y);
            if sx < 0 || sy < 0 || sx as usize >= w || sy as usize >= h {
                continue;
            }
            let i = sy as usize * w + sx as usize;
            let a = (a as u32 * k) >> 8;
            out[i] = if light { add(out[i], c, a) } else { over(out[i], c, a) };
        }
    }
}


/// VT323, a terminal face (SIL OFL 1.1, see fonts/README.md).
pub const VT323: &[u8] = include_bytes!("../fonts/VT323-Regular.ttf");
/// Permanent Marker, a brush marker face (Apache 2.0, see fonts/README.md).
pub const PERMANENT_MARKER: &[u8] = include_bytes!("../fonts/PermanentMarker-Regular.ttf");

pub fn font_terminal() -> Font {
    Font::from_bytes(VT323, FontSettings::default()).expect("built-in VT323")
}

pub fn font_marker() -> Font {
    Font::from_bytes(PERMANENT_MARKER, FontSettings::default()).expect("built-in Permanent Marker")
}

/// Colours the console swirl flickers through.
pub const SHADES_CHAOS: [u32; 6] = [0x14806A, 0x1FBF5A, 0x39FF6A, 0xB8FF6A, 0x2DE2C0, 0x0E5E4A];

/// The title's letters made sharper and wilder: stretched sideways, leaned
/// forward, wavering up and down along the line, and every stroke's ends
/// torn into sharp points (long ones trailing to the right, as if slashed
/// at speed; short ones to the left).
pub fn slash_letters(raw: &Mask, stretch: f32, lean: f32, s: f32) -> Mask {
    let wobble = 7.0 * s;
    let tail = (46.0 * s).max(4.0);
    let pad = (tail * 1.3) as usize + 4;
    let extra = (raw.h as f32 * lean).ceil() as usize;
    let sw = (raw.w as f32 * stretch) as usize;
    let (w, h) = (sw + extra + 2 * pad, raw.h + 2 * (wobble as usize + 2));
    let mut m = Mask::new(w, h);
    for y in 0..raw.h {
        for x in 0..sw {
            let v = raw.at(((x as f32 / stretch) as usize).min(raw.w - 1), y);
            if v == 0 {
                continue;
            }
            let xf = x as f32;
            let dy = (xf * 0.011).sin() * wobble * 0.6 + (xf * 0.031 + 1.3).sin() * wobble * 0.4;
            let yy = (y as f32 + dy + wobble + 2.0) as usize;
            let xx = x + pad + ((raw.h - y) as f32 * lean) as usize;
            if yy < h && xx < w {
                m.a[yy * w + xx] = m.a[yy * w + xx].max(v);
            }
        }
    }
    // Torn ends: each run of ink in a row gets a pointed spike off each end.
    // Spikes come in teeth a few rows tall, so the edges read as jagged
    // points rather than fuzz.
    let tooth = (13.0 * s).max(4.0) as usize;
    let src = m.a.clone();
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let t = y / tooth;
        let phase = (y % tooth) as f32 / tooth as f32;
        let shape = 1.0 - (phase - 0.5).abs() * 2.0;
        let r = hash(t as u32, 77) as f32 / u32::MAX as f32;
        let l = hash(t as u32, 91) as f32 / u32::MAX as f32;
        // About half the teeth get a blade; the rest leave the end as it is.
        let right = if r > 0.45 { (tail * shape * (r - 0.3)) as usize } else { 0 };
        let left = if l > 0.75 { (tail * 0.4 * shape * (l - 0.5)) as usize } else { 0 };
        let mut x = 0;
        while x < w {
            if row[x] < 140 {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && row[x] >= 140 {
                x += 1;
            }
            if x - start < 3 {
                continue;
            }
            for k in 0..right {
                let i = x + k;
                if i >= w || src[y * w + i] >= 140 {
                    break;
                }
                m.a[y * w + i] = 255;
            }
            for k in 1..=left {
                if start < k || src[y * w + start - k] >= 140 {
                    break;
                }
                m.a[y * w + start - k] = 255;
            }
        }
    }
    crop(&m)
}
