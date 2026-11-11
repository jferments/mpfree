//! The console theme's startup screen: a chaotic swirl of terminal
//! characters that slows, spirals in and settles into "MPFree OS v0.1" in
//! shaded ASCII art.
//!
//! The letters are drawn as a grid of terminal cells (VT323, a terminal
//! face). Each cell of the final picture is shaded twice: its *character* is
//! picked from a long ramp by how much ink and light it has (` .:-=+*#%@`
//! and many in between), and its *colour* from a ramp from deep teal in the
//! shadows through phosphor green to pale mint where the light catches the
//! upper-left edges. A drop shadow of dim characters sits under the letters.
//!
//! Timeline:
//! - 0.0 s: thousands of characters swirl round the centre, flickering
//!   through random characters and colours (inner ones turn faster).
//! - 1.0 s: cells start to spiral in to their places, the outer letters
//!   last; each one stops flickering and locks with a flash.
//! - 3.0 s: the spare characters fly out and fade; a prompt types.
//! - 3.9 s: settled; a slow light sweep and the odd glitching cell.
//!
//! Every particle's place and character come from the time and its seed,
//! so any frame can be drawn on its own.

use crate::gfx::{self, add, hash, hex, mix3, over, pack, scale, smoothstep, Mask};
use crate::Scene;
use mpfree_ui::SoftCanvas;
use std::f32::consts::{PI, TAU};

const CW: usize = 8;
const CH: usize = 13;
const BG: u32 = 0x020604;
/// Light to dark ink, for the shading.
const RAMP: &[u8] = b".'`^\",:;Il!i><~+_-?][}{1)(|/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$";
/// What the swirl flickers through.
const CHAOS: &[u8] = b"0123456789ABCDEF<>/\\|{}[]()#$%&*+=-~^:;!?@xXoO";
const PROMPT: &str = "> MPFree OS v0.1 :: console ready";

/// Shading colours, dark to light.
const SHADE: [u32; 7] = [0x0A2C3A, 0x0F6E6E, 0x16B26A, 0x39FF6A, 0x9BFF4A, 0xE6FF7A, 0xF4FFF0];

struct Particle {
    /// Where it ends: a cell, or None for a spare that flies away.
    target: Option<(usize, usize)>,
    ch: u8,
    color: u32,
    r0: f32,
    a0: f32,
    /// 0 settles first, 1 last.
    order: f32,
    seed: u32,
}

pub struct Console {
    w: usize,
    h: usize,
    cols: usize,
    rows: usize,
    glyphs: Vec<Mask>,
    parts: Vec<Particle>,
    bloom: Vec<u32>,
    prompt_row: usize,
}

fn glyph_index(c: u8) -> usize {
    (c.clamp(32, 126) - 32) as usize
}

/// The banner text as a mask at roughly `cols` cells wide; cells are taller
/// than wide, so the mask is sampled `CH / CW` times more coarsely down.
/// It fits in `cols` x `max_rows` cells; strokes are thickened first so the
/// terminal face's thin lines hold enough characters to shade.
fn banner(font: &fontdue::Font, text: &str, cols: usize, max_rows: usize) -> (Vec<f32>, usize, usize) {
    let m = gfx::text_mask(font, 260.0, 30.0, text).padded(6).dilated(4);
    let mut sx = m.w as f32 / cols as f32;
    if m.h as f32 / (sx * CH as f32 / CW as f32) > max_rows as f32 {
        sx = m.h as f32 / (max_rows as f32 * CH as f32 / CW as f32);
    }
    let cols = ((m.w as f32 / sx).ceil() as usize).max(1);
    let sy = sx * CH as f32 / CW as f32;
    let rows = ((m.h as f32 / sy).ceil() as usize).max(1);
    let mut cov = vec![0.0f32; cols * rows];
    for r in 0..rows {
        for c in 0..cols {
            let (x0, y0) = ((c as f32 * sx) as usize, (r as f32 * sy) as usize);
            let (x1, y1) = ((((c + 1) as f32 * sx) as usize).min(m.w), (((r + 1) as f32 * sy) as usize).min(m.h));
            let (mut sum, mut n) = (0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += m.at(x, y) as u32;
                    n += 1;
                }
            }
            cov[r * cols + c] = sum as f32 / (n.max(1) * 255) as f32;
        }
    }
    (cov, cols, rows)
}

/// A place on one of three spiral arms (radius, angle), so the swirl reads
/// as a vortex, with some scatter between the arms.
fn arm_place(next: &mut impl FnMut() -> u32) -> (f32, f32) {
    let u = (next() % 1000) as f32 / 1000.0;
    let r0 = 25.0 + u.powf(0.75) * 760.0;
    let arm = (next() % 3) as f32 * TAU / 3.0;
    let scatter = ((next() % 1000) as f32 / 1000.0 - 0.5) * if next().is_multiple_of(4) { 2.4 } else { 0.55 };
    (r0, arm + 2.2 * (r0 / 60.0).ln() + scatter)
}

impl Console {
    pub fn new(w: i32, h: i32) -> Console {
        let (w, h) = (w.max(CW as i32 * 4) as usize, h.max(CH as i32 * 4) as usize);
        let (cols, rows) = (w / CW, h / CH);
        let term = gfx::font_terminal();
        // Size the face so its advance fills a cell.
        let adv = term.metrics('M', 100.0).advance_width / 100.0;
        let px = (CW as f32 / adv).min(CH as f32 * 1.15);
        let lm = term.horizontal_line_metrics(px).expect("horizontal font");
        let base = ((CH as f32 + lm.ascent + lm.descent) / 2.0).round() as i32;
        let mut glyphs = Vec::new();
        for c in 32u8..=126 {
            let (m, bmp) = term.rasterize(c as char, px);
            let mut g = Mask::new(CW, CH);
            let ox = ((CW as i32 - m.width as i32) / 2).max(0);
            let oy = base - m.height as i32 - m.ymin;
            for y in 0..m.height {
                for x in 0..m.width {
                    let (gx, gy) = (ox + x as i32, oy + y as i32);
                    if gx >= 0 && gy >= 0 && (gx as usize) < CW && (gy as usize) < CH {
                        let v = bmp[y * m.width + x];
                        g.a[gy as usize * CW + gx as usize] = if v > 100 { 255 } else { (v as u32 * 2).min(255) as u8 };
                    }
                }
            }
            glyphs.push(g);
        }

        // The picture: "MPFree" big, "OS v0.1" under it, in terminal letters.
        let (big, bc, br) = banner(&term, crate::TITLE, cols * 86 / 100, rows * 46 / 100);
        let (small, sc, sr) = banner(&term, &format!("OS {}", crate::VERSION), cols * 56 / 100, rows * 24 / 100);
        let total = br + 2 + sr;
        let top = rows.saturating_sub(total + 4) / 2;
        let mut grid = vec![0.0f32; cols * rows];
        for (cov, bw, bh, c0, r0) in [(&big, bc, br, (cols - bc) / 2, top), (&small, sc, sr, (cols - sc) / 2, top + br + 2)] {
            for r in 0..bh {
                for c in 0..bw {
                    let (gc, gr) = (c0 + c, r0 + r);
                    if gc < cols && gr < rows {
                        grid[gr * cols + gc] = cov[r * bw + c];
                    }
                }
            }
        }
        // Light from the upper left: the slope of a softened copy, so edges
        // facing the light are bright and the others fall into shadow.
        let soft = {
            let mut m = Mask::new(cols, rows);
            for (i, v) in grid.iter().enumerate() {
                m.a[i] = (v * 255.0) as u8;
            }
            m.blurred(1, 2)
        };
        let sv = |c: i32, r: i32| -> f32 {
            if c < 0 || r < 0 || c as usize >= cols || r as usize >= rows {
                0.0
            } else {
                soft.a[r as usize * cols + c as usize] as f32 / 255.0
            }
        };
        let mut parts = Vec::new();
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let (cx, cy) = (cols as f32 / 2.0, rows as f32 / 2.0);
        let max_d = (cx * cx + (cy * 1.6).powi(2)).sqrt();
        for r in 0..rows {
            for c in 0..cols {
                let v = grid[r * cols + c];
                let (ci, ri) = (c as i32, r as i32);
                let shadow = v < 0.12 && r > 0 && c > 0 && grid[(r - 1) * cols + c - 1] > 0.4;
                if v < 0.12 && !shadow {
                    continue;
                }
                let (ch, color) = if shadow {
                    (b".:;"[(next() % 3) as usize], SHADE[0])
                } else {
                    let light = ((sv(ci + 1, ri + 1) - sv(ci - 1, ri - 1)) * 1.6).clamp(-1.0, 1.0);
                    let depth = sv(ci, ri);
                    // How much ink: coverage, deeper inside, brighter towards the light.
                    let ink = (v * 0.55 + depth * 0.45 + light * 0.25).clamp(0.0, 1.0);
                    let jitter = (next() % 7) as i32 - 3;
                    let k = ((ink * (RAMP.len() - 1) as f32) as i32 + jitter).clamp(0, RAMP.len() as i32 - 1) as usize;
                    let tone = (0.15 + ink * 0.55 + light * 0.45).clamp(0.0, 0.999) * (SHADE.len() - 1) as f32;
                    let (i, f) = (tone as usize, tone.fract());
                    let col = pack(mix3(hex(SHADE[i]), hex(SHADE[(i + 1).min(SHADE.len() - 1)]), f));
                    (RAMP[k], col)
                };
                let d = (((c as f32 - cx).powi(2) + ((r as f32 - cy) * 1.6).powi(2)).sqrt() / max_d).min(1.0);
                let (r0, a0) = arm_place(&mut next);
                parts.push(Particle { target: Some((c, r)), ch, color, r0, a0, order: d * 0.8 + (next() % 200) as f32 / 1000.0, seed: next() });
            }
        }
        for _ in 0..2200 {
            let (r0, a0) = arm_place(&mut next);
            let color = SHADE[1 + (next() % 4) as usize];
            parts.push(Particle { target: None, ch: b'#', color, r0, a0, order: (next() % 1000) as f32 / 1000.0, seed: next() });
        }
        let prompt_row = (top + total + 4).min(rows.saturating_sub(2));
        Console { w, h, cols, rows, glyphs, parts, bloom: Vec::new(), prompt_row }
    }

    fn put(&self, px: &mut [u32], x: i32, y: i32, ch: u8, col: u32, k: u32) {
        let g = &self.glyphs[glyph_index(ch)];
        for gy in 0..CH {
            let yy = y + gy as i32;
            if yy < 0 || yy as usize >= self.h {
                continue;
            }
            let row = yy as usize * self.w;
            for gx in 0..CW {
                let a = g.a[gy * CW + gx] as u32;
                if a == 0 {
                    continue;
                }
                let xx = x + gx as i32;
                if xx < 0 || xx as usize >= self.w {
                    continue;
                }
                px[row + xx as usize] = over(px[row + xx as usize], col, (a * k) >> 8);
            }
        }
    }
}

impl Scene for Console {
    fn length(&self) -> f32 {
        4.2
    }

    fn draw(&mut self, t: f32, out: &mut SoftCanvas) {
        let (w, h) = (self.w, self.h);
        let px = &mut out.px;
        px.fill(BG);
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let fade_in = smoothstep(0.0, 0.35, t);
        let step = (t * 20.0) as u32;
        for p in &self.parts {
            let spin = 1.1 + 1.2 / (0.4 + p.r0 / 260.0);
            let swirl_a = p.a0 + spin * t + 0.25 * (t * 1.7 + p.order * 9.0).sin();
            let wobble = 1.0 + 0.08 * (t * 2.3 + p.a0 * 3.0).sin();
            let (sx, sy) = (cx + swirl_a.cos() * p.r0 * wobble * 1.45, cy + swirl_a.sin() * p.r0 * wobble * 0.82);
            let flick = CHAOS[(hash(p.seed, step) as usize) % CHAOS.len()];
            let flick_col = gfx::SHADES_CHAOS[(hash(p.seed ^ 0xABCD, step / 2) as usize) % gfx::SHADES_CHAOS.len()];
            match p.target {
                None => {
                    // Spares fly outwards and fade as the picture settles.
                    let go = smoothstep(2.2 + p.order * 0.6, 3.3 + p.order * 0.6, t);
                    if go >= 1.0 {
                        continue;
                    }
                    let push = 1.0 + go * 1.6;
                    let (x, y) = (cx + (sx - cx) * push, cy + (sy - cy) * push);
                    let k = ((1.0 - go) * fade_in * 150.0) as u32;
                    self.put(px, x as i32, y as i32, flick, flick_col, k);
                }
                Some((c, r)) => {
                    let start = 1.0 + p.order * 1.5;
                    let e = smoothstep(start, start + 0.9, t);
                    let (tx, ty) = ((c * CW) as f32, (r * CH) as f32);
                    let (x, y, ch, col, k) = if e >= 1.0 {
                        let since = t - (start + 0.9);
                        let flash = 1.0 - smoothstep(0.0, 0.25, since);
                        let col = if flash > 0.0 { pack(mix3(hex(p.color), hex(0xFFFFFF), flash * 0.8)) } else { p.color };
                        // After it settles, a slow light sweep and the odd glitch.
                        let sweep = ((tx / w as f32) * 3.0 - (t - 3.6) * 1.2).rem_euclid(3.0);
                        let lift = if sweep < 0.25 { (1.0 - (sweep - 0.125).abs() * 8.0) * 0.35 } else { 0.0 };
                        let col = if lift > 0.0 { pack(mix3(hex(col), hex(0xEFFFE6), lift)) } else { col };
                        let glitch = t > 3.8 && hash(p.seed, (t * 8.0) as u32).is_multiple_of(1500);
                        (tx, ty, if glitch { flick } else { p.ch }, col, 256)
                    } else {
                        // Spiral in: radius and angle move from the swirl to the place.
                        let (dx, dy) = ((tx - cx) / 1.45, (ty - cy) / 0.82);
                        let rt = (dx * dx + dy * dy).sqrt();
                        let at = dy.atan2(dx);
                        let mut diff = (swirl_a - at).rem_euclid(TAU);
                        if diff > PI {
                            diff -= TAU;
                        }
                        let ee = e * e * (3.0 - 2.0 * e);
                        let a = at + diff * (1.0 - ee);
                        let rr = p.r0 * wobble + (rt - p.r0 * wobble) * ee;
                        let (x, y) = (cx + a.cos() * rr * 1.45, cy + a.sin() * rr * 0.82);
                        let calm = e > 0.6 && !hash(p.seed, step).is_multiple_of(3);
                        let ch = if calm { p.ch } else { flick };
                        let col = if e > 0.4 { p.color } else { flick_col };
                        (x, y, ch, col, (fade_in * (180.0 + 76.0 * e)) as u32)
                    };
                    self.put(px, x as i32, y as i32, ch, col, k);
                }
            }
        }
        // The prompt types once the picture has settled.
        if t > 3.3 {
            let n = (((t - 3.3) * 40.0) as usize).min(PROMPT.len());
            let row = self.prompt_row;
            let c0 = (self.cols.saturating_sub(PROMPT.len())) / 2;
            for (i, b) in PROMPT.bytes().take(n).enumerate() {
                if b != b' ' {
                    self.put(px, ((c0 + i) * CW) as i32, (row * CH) as i32, b, 0x7CFF9A, 230);
                }
            }
            if n < PROMPT.len() || ((t * 2.5) as u32).is_multiple_of(2) {
                self.put(px, ((c0 + n) * CW) as i32, (row * CH) as i32, b'_', 0xEFFFE6, 256);
            }
        }
        let _ = self.rows;

        // Bloom: a quarter-size blurred copy added back, then light scanlines.
        let (bw, bh) = (w / 4, h / 4);
        let mut small = Mask::new(bw * 3, bh);
        for y in 0..bh {
            for x in 0..bw {
                let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
                for yy in 0..4 {
                    let row = (y * 4 + yy) * w + x * 4;
                    for p in &px[row..row + 4] {
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
        let blur = small.blurred(3, 2);
        self.bloom.resize(bw * bh, 0);
        for y in 0..bh {
            let i = y * blur.w;
            for x in 0..bw {
                self.bloom[y * bw + x] = gfx::rgb(blur.a[i + x] as u32, blur.a[i + bw + x] as u32, blur.a[i + 2 * bw + x] as u32);
            }
        }
        for y in 0..h {
            let by = (y / 4).min(bh - 1);
            let row = &mut px[y * w..(y + 1) * w];
            let dark = y % 3 == 2;
            for (x, p) in row.iter_mut().enumerate() {
                let mut v = add(*p, self.bloom[by * bw + (x / 4).min(bw - 1)], 320);
                if dark {
                    v = scale(v, 200);
                }
                *p = v;
            }
        }
    }
}
