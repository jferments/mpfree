//! The console theme's startup screen: green ASCII chaos that resolves into
//! the name.
//!
//! The screen is a grid of character cells drawn at half resolution and
//! doubled (chunky pixels), with scanlines and a phosphor bloom added while
//! doubling. Timeline:
//!
//! - 0.0 s: a prompt types the boot command.
//! - 0.3 s: rain and noise fill the screen: random characters flicker
//!   everywhere and streams fall down the columns.
//! - 1.3 s: the cells of the banner start to "decrypt", left to right: each
//!   one scrambles brightly, then locks to its final ASCII-art character
//!   with a white flash.
//! - 2.9 s: the rain stops and the noise fades, leaving the banner glowing;
//!   the subtitle and a short log type out underneath.
//!
//! Every cell's state is worked out from the time and its place (a hash),
//! so any frame can be drawn on its own.

use crate::gfx::{self, add, hash, over, scale, smoothstep, Mask};
use crate::Scene;
use mpfree_ui::SoftCanvas;

/// Cell size at half resolution (doubled on screen: 12 x 20).
const CW: usize = 6;
const CH: usize = 10;
/// The characters the chaos uses.
const CHAOS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%&*+-=<>/\\|{}[]()?!@^~:;";
/// Light to dark, for the ASCII art.
const RAMP: &[u8] = b".:-=+*#%@";

const BG: u32 = 0x010702;
const DIM: [f32; 3] = gfx::hex(0x0B4A1C);
const GREEN: [f32; 3] = gfx::hex(0x39FF6A);
const HOT: [f32; 3] = gfx::hex(0xE4FFE9);

const PROMPT: &str = "mpfree@mpc-one:~$ boot --theme console";
const SUBTITLE: &str = "O S   v 0 . 1   ::   open firmware for the MPC One";
const LOG: [&str; 4] = ["> unscramble ..................... ok", "> load theme ..................... console",
                        "> hand over to the pad player", "> _"];

pub struct Console {
    w: usize,
    h: usize,
    cols: usize,
    rows: usize,
    glyphs: Vec<Mask>,
    /// Banner cells: (column, row, final character, lock time).
    banner: Vec<(usize, usize, u8, f32)>,
    /// For each cell, the index into `banner` or usize::MAX.
    banner_at: Vec<usize>,
    low: Vec<u32>,
    bloom: Vec<u32>,
}

fn glyph_index(c: u8) -> usize {
    (c.clamp(32, 126) - 32) as usize
}

impl Console {
    pub fn new(w: i32, h: i32) -> Console {
        let (w, h) = (w.max(2) as usize, h.max(2) as usize);
        let (lw, lh) = (w / 2, h / 2);
        let (cols, rows) = (lw / CW, lh / CH);
        // The glyphs, rasterised for 6 x 10 cells and made crisp.
        let mono = gfx::font_mono();
        let mut glyphs = Vec::new();
        for c in 32u8..=126 {
            let (m, bmp) = mono.rasterize(c as char, 10.5);
            let mut g = Mask::new(CW, CH);
            let ox = ((CW as i32 - m.width as i32) / 2).max(0);
            let oy = 8 - m.height as i32 - m.ymin;
            for y in 0..m.height {
                for x in 0..m.width {
                    let (gx, gy) = (ox + x as i32, oy + y as i32);
                    if gx >= 0 && gy >= 0 && (gx as usize) < CW && (gy as usize) < CH {
                        let v = bmp[y * m.width + x];
                        g.a[gy as usize * CW + gx as usize] = if v > 110 { 255 } else if v > 50 { 120 } else { 0 };
                    }
                }
            }
            glyphs.push(g);
        }
        // The banner: "MPFree" sampled onto the cell grid, cells taller than
        // wide, each cell's ink picking a character from the ramp.
        let sans = gfx::font_sans();
        let m = gfx::text_mask(&sans, 240.0, 10.0, crate::TITLE);
        let bcols = (cols * 80 / 100).max(1);
        let sx = m.w as f32 / bcols as f32;
        let sy = sx * CH as f32 / CW as f32;
        let brows = ((m.h as f32 / sy).ceil() as usize).max(1);
        let (c0, r0) = ((cols - bcols) / 2, (rows.saturating_sub(brows)) * 2 / 5);
        let mut banner = Vec::new();
        let mut banner_at = vec![usize::MAX; cols * rows];
        for br in 0..brows {
            for bc in 0..bcols {
                let (x0, y0) = ((bc as f32 * sx) as usize, (br as f32 * sy) as usize);
                let (x1, y1) = ((((bc + 1) as f32 * sx) as usize).min(m.w), (((br + 1) as f32 * sy) as usize).min(m.h));
                let mut sum = 0u32;
                let mut n = 0u32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        sum += m.at(x, y) as u32;
                        n += 1;
                    }
                }
                let cov = sum as f32 / (n.max(1) * 255) as f32;
                if cov < 0.07 {
                    continue;
                }
                let ch = RAMP[((cov * RAMP.len() as f32) as usize).min(RAMP.len() - 1)];
                let (c, r) = (c0 + bc, r0 + br);
                if c >= cols || r >= rows {
                    continue;
                }
                // Left to right with some scatter.
                let jitter = (hash(c as u32, r as u32 + 999) % 1000) as f32 / 1000.0;
                let lock = 1.4 + 1.1 * (bc as f32 / bcols as f32) + 0.45 * jitter;
                banner_at[r * cols + c] = banner.len();
                banner.push((c, r, ch, lock));
            }
        }
        Console { w, h, cols, rows, glyphs, banner, banner_at, low: vec![BG; lw * lh], bloom: Vec::new() }
    }

    fn banner_rows(&self) -> (usize, usize) {
        let lo = self.banner.iter().map(|b| b.1).min().unwrap_or(0);
        let hi = self.banner.iter().map(|b| b.1).max().unwrap_or(0);
        (lo, hi)
    }

    /// One cell: character and brightness (0 = dark, 1 = green, up to 2 = white hot).
    fn cell(&self, c: usize, r: usize, t: f32) -> Option<(u8, f32)> {
        let step = (t * 14.0) as u32;
        let cell_id = (r * self.cols + c) as u32;
        let rnd = |salt: u32| hash(cell_id ^ salt.wrapping_mul(0x01000193), step);
        // Banner cells.
        if let Some(&(_, _, ch, lock)) = self.banner.get(self.banner_at[r * self.cols + c]) {
            if t >= lock {
                let flash = 1.0 - smoothstep(0.0, 0.35, t - lock);
                let idle = 0.85 + 0.15 * ((t * 3.0 + c as f32 * 0.21).sin() * 0.5 + 0.5);
                return Some((ch, idle + flash * 1.1));
            }
            if t >= lock - 0.7 {
                let k = 0.55 + 0.45 * smoothstep(lock - 0.7, lock, t);
                let pick = CHAOS[(rnd(7) as usize) % CHAOS.len()];
                return Some((pick, k + (rnd(9) % 3) as f32 * 0.1));
            }
        }
        // Rain: a few drops per column, each falling once.
        let mut best: Option<(u8, f32)> = None;
        for k in 0..4u32 {
            let hk = hash(c as u32 + 77, k);
            let start = 0.25 + (hk % 2000) as f32 / 1000.0 + k as f32 * 0.1;
            if start > 2.4 {
                continue;
            }
            let speed = 16.0 + (hk >> 12) as f32 % 26.0;
            let len = 6.0 + (hk >> 20) as f32 % 18.0;
            let head = (t - start) * speed;
            let d = head - r as f32;
            if d < 0.0 || d > len {
                continue;
            }
            let fade = 1.0 - smoothstep(2.7, 3.4, t);
            if fade <= 0.0 {
                continue;
            }
            let k = fade * if d < 1.0 { 1.6 } else { 1.0 - d / len };
            let pick = CHAOS[(hash(cell_id, (t * 9.0) as u32 + k as u32) as usize) % CHAOS.len()];
            if best.is_none_or(|(_, b)| b < k) {
                best = Some((pick, k));
            }
        }
        // Noise: random flicker that swells and fades.
        let density = smoothstep(0.25, 1.0, t) * (1.0 - 0.5 * smoothstep(1.4, 2.0, t)) * (1.0 - smoothstep(2.5, 3.2, t));
        if density > 0.0 && (rnd(3) % 1000) as f32 / 1000.0 < density * 0.16 {
            let pick = CHAOS[(rnd(5) as usize) % CHAOS.len()];
            let k = 0.18 + (rnd(11) % 40) as f32 / 100.0;
            if best.is_none_or(|(_, b)| b < k) {
                best = Some((pick, k));
            }
        }
        best
    }

    fn put(&mut self, c: usize, r: usize, ch: u8, k: f32) {
        let lw = self.w / 2;
        let col = if k <= 1.0 { gfx::mix3(DIM, GREEN, k.clamp(0.0, 1.0)) } else { gfx::mix3(GREEN, HOT, (k - 1.0).min(1.0)) };
        let col = gfx::pack(col);
        let g = &self.glyphs[glyph_index(ch)];
        let (x0, y0) = (c * CW, r * CH);
        for y in 0..CH {
            let row = (y0 + y) * lw + x0;
            for x in 0..CW {
                let a = g.a[y * CW + x] as u32;
                if a != 0 {
                    self.low[row + x] = over(self.low[row + x], col, a + 1);
                }
            }
        }
    }

    fn text(&mut self, c: usize, r: usize, s: &str, k: f32) {
        for (i, b) in s.bytes().enumerate() {
            if c + i < self.cols && r < self.rows && b != b' ' {
                self.put(c + i, r, b, k);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// `s` typed from `start` at `cps` characters a second, with a cursor
    /// while typing (and blinking after, if `cursor`).
    fn typed(&mut self, c: usize, r: usize, s: &str, start: f32, cps: f32, t: f32, k: f32, cursor: bool) {
        if t < start {
            return;
        }
        let n = (((t - start) * cps) as usize).min(s.len());
        self.text(c, r, &s[..n], k);
        let typing = n < s.len();
        if (typing || cursor) && (((t * 2.5) as u32).is_multiple_of(2) || typing) && c + n < self.cols {
            self.put(c + n, r, b'#', 1.2);
        }
    }
}

impl Scene for Console {
    fn length(&self) -> f32 {
        4.6
    }

    fn draw(&mut self, t: f32, out: &mut SoftCanvas) {
        let (lw, lh) = (self.w / 2, self.h / 2);
        self.low.fill(BG);
        for r in 0..self.rows {
            for c in 0..self.cols {
                if let Some((ch, k)) = self.cell(c, r, t) {
                    self.put(c, r, ch, k);
                }
            }
        }
        // The prompt types at the top, then dims.
        let pk = 1.0 - 0.5 * smoothstep(1.0, 2.0, t);
        self.typed(1, 1, PROMPT, 0.0, 48.0, t, pk, false);
        let (_, last) = self.banner_rows();
        let sub = (self.cols.saturating_sub(SUBTITLE.len())) / 2;
        self.typed(sub, last + 3, SUBTITLE, 2.9, 60.0, t, 1.0, false);
        for (i, line) in LOG.iter().enumerate() {
            let at = 3.5 + i as f32 * 0.22;
            let r = self.rows.saturating_sub(LOG.len() + 2) + i;
            self.typed(2, r, line, at, 90.0, t, 0.75, i == LOG.len() - 1);
        }

        // Bloom: a quarter-size blurred copy, added back while doubling.
        let (bw, bh) = (lw / 4, lh / 4);
        let mut small = gfx::Mask::new(bw * 3, bh); // r, g, b side by side
        for y in 0..bh {
            for x in 0..bw {
                let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
                for yy in 0..4 {
                    for xx in 0..4 {
                        let p = self.low[(y * 4 + yy) * lw + x * 4 + xx];
                        r += (p >> 16) & 255;
                        g += (p >> 8) & 255;
                        b += p & 255;
                    }
                }
                small.a[y * small.w + x] = (r / 16) as u8;
                small.a[y * small.w + bw + x] = (g / 16) as u8;
                small.a[y * small.w + 2 * bw + x] = (b / 16) as u8;
            }
        }
        let blur = small.blurred(2, 2);
        self.bloom.resize(bw * bh, 0);
        for y in 0..bh {
            for x in 0..bw {
                let i = y * blur.w;
                self.bloom[y * bw + x] = gfx::rgb(blur.a[i + x] as u32, blur.a[i + bw + x] as u32, blur.a[i + 2 * bw + x] as u32);
            }
        }

        // Glitches: a few short moments where bands of rows slip sideways.
        let glitch = [(1.05, 0.07), (1.95, 0.05), (2.45, 0.08), (4.2, 0.04)]
            .iter().any(|&(at, len)| t >= at && t < at + len);
        let gstep = (t * 60.0) as u32;

        let w = self.w;
        let px = &mut out.px;
        for y in 0..self.h.min(px.len() / w) {
            let ly = (y / 2).min(lh - 1);
            let shift = if glitch && hash(ly as u32 / 6, gstep).is_multiple_of(5) { (hash(ly as u32, gstep) % 40) as i32 - 20 } else { 0 };
            let dark = y % 2 == 1;
            let by = (ly / 4).min(bh - 1);
            for x in 0..w {
                let lx = ((x / 2) as i32 + shift).clamp(0, lw as i32 - 1) as usize;
                let mut p = self.low[ly * lw + lx];
                let bx = (lx / 4).min(bw - 1);
                p = add(p, self.bloom[by * bw + bx], 300);
                if dark {
                    p = scale(p, 150);
                }
                px[y * w + x] = p;
            }
        }
    }
}
