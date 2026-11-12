//! The console theme's startup screen: a chaotic swirl of terminal
//! characters that slows, spirals in and settles into "MPFree OS v0.1" in
//! shaded ASCII art.
//!
//! The title is the blade letters of the other startup screens (see
//! [`crate::blade`]), drawn at full size and then turned into a grid of
//! terminal cells (VT323 characters), one character per cell and none
//! overlapping. Each cell's *character* is the one whose own shape and
//! density best match the ink in that cell, so edges follow the letters and
//! the lit side gets denser characters than the shadow side. Its *colour*
//! runs from pale mint at the top of the letters through phosphor green to
//! teal at the bottom, lifted where the light from the upper left falls.
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

use crate::blade;
use crate::gfx::{self, add, hash, hex, mix3, over, pack, scale, smoothstep, Mask};
use crate::Scene;
use mpfree_ui::SoftCanvas;
use std::f32::consts::{PI, TAU};

const CW: usize = 8;
const CH: usize = 13;
/// Each cell is matched to a character as a SX x SY grid of ink.
const SX: usize = 4;
const SY: usize = 6;
/// The characters the picture may use (space included, for empty cells).
const PALETTE: &[u8] =
    b" .,:;'`\"-_~=+*^<>/\\|()[]{}!?ilIjtfrxnuvczoaesYTLJCUQOZXVAKHNMWE#%&@$80db";
const BG: u32 = 0x020604;
/// What the swirl flickers through.
const CHAOS: &[u8] = b"0123456789ABCDEF<>/\\|{}[]()#$%&*+=-~^:;!?@xXoO";
const PROMPT: &str = "> MPFree OS v0.1 :: console ready";

/// Shading colours, dark to light.
const SHADE: [u32; 7] = [
    0x0A2C3A, 0x0F6E6E, 0x16B26A, 0x39FF6A, 0x9BFF4A, 0xE6FF7A, 0xF4FFF0,
];

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

/// A place on one of three spiral arms (radius, angle), so the swirl reads
/// as a vortex, with some scatter between the arms.
fn arm_place(next: &mut impl FnMut() -> u32) -> (f32, f32) {
    let u = (next() % 1000) as f32 / 1000.0;
    let r0 = 25.0 + u.powf(0.75) * 760.0;
    let arm = (next() % 3) as f32 * TAU / 3.0;
    let scatter =
        ((next() % 1000) as f32 / 1000.0 - 0.5) * if next().is_multiple_of(4) { 2.4 } else { 0.55 };
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
                        g.a[gy as usize * CW + gx as usize] = if v > 100 {
                            255
                        } else {
                            (v as u32 * 2).min(255) as u8
                        };
                    }
                }
            }
            glyphs.push(g);
        }

        // The picture: "MPFree OS" and "v0.1" in the blade letters the other
        // startup screens use, drawn at full size, then turned into
        // characters cell by cell. Each cell gets the character whose own
        // shape best matches the ink in that cell (so edges follow the
        // letters with / \ | _ and the like), scaled by how lit the cell is
        // (so the light side gets denser characters than the shadow side).
        let s = h as f32 / 800.0;
        let title = format!("{} OS", crate::TITLE);
        let style = blade::Style {
            seed: 3,
            weight: 1.45,
            ..Default::default()
        };
        let mut cap = 190.0 * s;
        let mut tm = blade::render(&title, cap, style);
        let max_w = w as f32 * 0.88;
        if tm.w as f32 > max_w {
            cap *= max_w / tm.w as f32;
            tm = blade::render(&title, cap, style);
        }
        let vm = blade::render(crate::VERSION, cap * 0.5, blade::Style { seed: 5, ..style });
        let gap = CH * 2;
        let total = tm.h + gap + vm.h;
        let top = (h.saturating_sub(total + CH * 6) / 2) / CH * CH;
        let mut ink = Mask::new(w, h);
        let mut place = |m: &Mask, y: usize| {
            let x0 = (w.saturating_sub(m.w) / 2) / CW * CW;
            for yy in 0..m.h.min(h.saturating_sub(y)) {
                for xx in 0..m.w.min(w - x0) {
                    ink.a[(y + yy) * w + x0 + xx] = m.a[yy * m.w + xx];
                }
            }
        };
        place(&tm, top);
        let vtop = (top + tm.h + gap) / CH * CH;
        place(&vm, vtop);
        let title_rows = (top / CH, (top + tm.h).div_ceil(CH));
        // Each cell as a SX x SY grid of ink amounts, and every character's
        // shape the same way.
        let feat = |m: &Mask, x0: usize, y0: usize, out: &mut [f32; SX * SY]| {
            for j in 0..SY {
                for i in 0..SX {
                    let (ax, bx) = (x0 + i * CW / SX, x0 + (i + 1) * CW / SX);
                    let (ay, by) = (y0 + j * CH / SY, y0 + (j + 1) * CH / SY);
                    let mut sum = 0u32;
                    for y in ay..by {
                        for x in ax..bx {
                            sum += m.a[y * m.w + x] as u32;
                        }
                    }
                    out[j * SX + i] = sum as f32 / (((bx - ax) * (by - ay)).max(1) * 255) as f32;
                }
            }
        };
        // Each character's density, and its shape with the density taken
        // out (mean removed, scaled to unit spread), so the two can be
        // matched separately.
        let shape_of = |f: &[f32; SX * SY]| {
            let n = f.len() as f32;
            let mean = f.iter().sum::<f32>() / n;
            let sd = (f.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n).sqrt();
            let mut out = [0.0f32; SX * SY];
            if sd > 0.02 {
                for (o, v) in out.iter_mut().zip(f) {
                    *o = (v - mean) / sd;
                }
            }
            (mean, out)
        };
        let shapes: Vec<(u8, f32, [f32; SX * SY])> = PALETTE
            .iter()
            .map(|&c| {
                let mut f = [0.0; SX * SY];
                feat(&glyphs[glyph_index(c)], 0, 0, &mut f);
                let (d, sh) = shape_of(&f);
                (c, d, sh)
            })
            .collect();
        let densest = shapes.iter().map(|s| s.1).fold(0.0f32, f32::max);
        let mut cells = vec![[0.0f32; SX * SY]; cols * rows];
        let mut grid = vec![0.0f32; cols * rows];
        for r in 0..rows {
            for c in 0..cols {
                let f = &mut cells[r * cols + c];
                feat(&ink, c * CW, r * CH, f);
                grid[r * cols + c] = f.iter().sum::<f32>() / f.len() as f32;
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
                if v < 0.04 {
                    continue;
                }
                let (ch, color) = {
                    let light = ((sv(ci + 1, ri + 1) - sv(ci - 1, ri - 1)) * 1.6).clamp(-1.0, 1.0);
                    // How dense the characters get: lit cells darker ink, the
                    // shadow side lighter.
                    let (_, want) = shape_of(&cells[r * cols + c]);
                    let target = densest * v * (0.6 + 0.4 * light).clamp(0.2, 1.0);
                    let mut best = (f32::MAX, b' ');
                    for (g, d, f) in &shapes {
                        let shape: f32 = want
                            .iter()
                            .zip(f)
                            .map(|(a, b)| (a - b).powi(2))
                            .sum::<f32>();
                        let e = (d - target).powi(2) * 400.0 + shape * 0.02;
                        if e < best.0 {
                            best = (e, *g);
                        }
                    }
                    if best.1 == b' ' {
                        continue;
                    }
                    // Colour: pale mint at the top of the letters to deep
                    // teal at the bottom, lifted where the light falls.
                    let (r0, r1) = title_rows;
                    let down = if (r0..r1).contains(&r) {
                        (r - r0) as f32 / (r1 - r0).max(1) as f32
                    } else {
                        0.35
                    };
                    let tone = (0.2 + (1.0 - down) * 0.6 + light * 0.3 + v * 0.15)
                        .clamp(0.0, 0.999)
                        * (SHADE.len() - 1) as f32;
                    let (i, f) = (tone as usize, tone.fract());
                    let col = pack(mix3(
                        hex(SHADE[i]),
                        hex(SHADE[(i + 1).min(SHADE.len() - 1)]),
                        f,
                    ));
                    (best.1, col)
                };
                let d = (((c as f32 - cx).powi(2) + ((r as f32 - cy) * 1.6).powi(2)).sqrt()
                    / max_d)
                    .min(1.0);
                let (r0, a0) = arm_place(&mut next);
                parts.push(Particle {
                    target: Some((c, r)),
                    ch,
                    color,
                    r0,
                    a0,
                    order: d * 0.8 + (next() % 200) as f32 / 1000.0,
                    seed: next(),
                });
            }
        }
        for _ in 0..2200 {
            let (r0, a0) = arm_place(&mut next);
            let color = SHADE[1 + (next() % 4) as usize];
            parts.push(Particle {
                target: None,
                ch: b'#',
                color,
                r0,
                a0,
                order: (next() % 1000) as f32 / 1000.0,
                seed: next(),
            });
        }
        let prompt_row = ((vtop + vm.h) / CH + 3).min(rows.saturating_sub(2));
        Console {
            w,
            h,
            cols,
            rows,
            glyphs,
            parts,
            bloom: Vec::new(),
            prompt_row,
        }
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
            let (sx, sy) = (
                cx + swirl_a.cos() * p.r0 * wobble * 1.45,
                cy + swirl_a.sin() * p.r0 * wobble * 0.82,
            );
            let flick = CHAOS[(hash(p.seed, step) as usize) % CHAOS.len()];
            let flick_col = gfx::SHADES_CHAOS
                [(hash(p.seed ^ 0xABCD, step / 2) as usize) % gfx::SHADES_CHAOS.len()];
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
                        let col = if flash > 0.0 {
                            pack(mix3(hex(p.color), hex(0xFFFFFF), flash * 0.8))
                        } else {
                            p.color
                        };
                        // After it settles, a slow light sweep and the odd glitch.
                        let sweep = ((tx / w as f32) * 3.0 - (t - 3.6) * 1.2).rem_euclid(3.0);
                        let lift = if sweep < 0.25 {
                            (1.0 - (sweep - 0.125).abs() * 8.0) * 0.35
                        } else {
                            0.0
                        };
                        let col = if lift > 0.0 {
                            pack(mix3(hex(col), hex(0xEFFFE6), lift))
                        } else {
                            col
                        };
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
                    self.put(
                        px,
                        ((c0 + i) * CW) as i32,
                        (row * CH) as i32,
                        b,
                        0x7CFF9A,
                        230,
                    );
                }
            }
            if n < PROMPT.len() || ((t * 2.5) as u32).is_multiple_of(2) {
                self.put(
                    px,
                    ((c0 + n) * CW) as i32,
                    (row * CH) as i32,
                    b'_',
                    0xEFFFE6,
                    256,
                );
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
                self.bloom[y * bw + x] = gfx::rgb(
                    blur.a[i + x] as u32,
                    blur.a[i + bw + x] as u32,
                    blur.a[i + 2 * bw + x] as u32,
                );
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
