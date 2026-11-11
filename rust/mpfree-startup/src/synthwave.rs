//! The synthwave theme's startup screen: a striped sun rising behind neon
//! mountains and palm trees, over a glowing grid racing towards the viewer,
//! and a chrome "MPFree" that flickers on like a neon tube.
//!
//! Layers made once: the sky with its stars, the sun's halo, the mountains,
//! the palm trees and the title. A frame copies the sky, places the sun at
//! its height, draws the moving grid (a few spans per row) and lays the
//! still layers over it.
//!
//! - 0.0 s: the sun rises from below the horizon while the grid starts.
//! - 1.5 s: the title flickers on, then the subtitle.
//! - 3.2 s: settled; the grid keeps moving and the stars twinkle.

use crate::gfx::{self, add, hex, mix3, over, pack, smoothstep, Mask, Rng, Sprite};
use crate::Scene;
use mpfree_ui::SoftCanvas;

const PINK: u32 = 0xFF2E97;
const CYAN: u32 = 0x2DE2E6;

pub struct Synthwave {
    w: usize,
    h: usize,
    horizon: usize,
    sky: Vec<u32>,
    stars: Vec<(usize, u32, f32)>,
    sun_r: f32,
    halo: Mask,
    mountains: Sprite,
    rim_glow: Sprite,
    palms: Sprite,
    title: Sprite,
    title_glow: Sprite,
    sub: Sprite,
    sub_glow: Sprite,
}

/// Fill a disc of radius `r` at (cx, cy) into `m` (anti-aliased, max).
fn disc(m: &mut Mask, cx: f32, cy: f32, r: f32) {
    let (x0, x1) = ((cx - r - 1.0).max(0.0) as usize, ((cx + r + 2.0) as usize).min(m.w));
    let (y0, y1) = ((cy - r - 1.0).max(0.0) as usize, ((cy + r + 2.0) as usize).min(m.h));
    for y in y0..y1 {
        for x in x0..x1 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let a = ((r - d + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
            let i = y * m.w + x;
            m.a[i] = m.a[i].max(a);
        }
    }
}

/// A stroke along `pts` whose radius goes from `r0` to `r1`.
fn stroke(m: &mut Mask, pts: &[(f32, f32)], r0: f32, r1: f32) {
    let n = pts.len().max(2) - 1;
    for i in 0..n {
        let (a, b) = (pts[i], pts[i + 1]);
        let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) / 1.5).ceil().max(1.0) as usize;
        for s in 0..=steps {
            let f = s as f32 / steps as f32;
            let r = r0 + (r1 - r0) * ((i as f32 + f) / n as f32);
            disc(m, a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, r);
        }
    }
}

/// A palm tree: a curved trunk from (bx, by) leaning by `lean`, `height`
/// tall, and a crown of drooping, feathered fronds.
fn palm(m: &mut Mask, bx: f32, by: f32, height: f32, lean: f32, seed: u32) {
    let mut rng = Rng(seed);
    let top = (bx + lean, by - height);
    let ctrl = (bx + lean * 0.15, by - height * 0.55);
    let trunk: Vec<(f32, f32)> = (0..=40).map(|i| {
        let t = i as f32 / 40.0;
        let u = 1.0 - t;
        (u * u * bx + 2.0 * u * t * ctrl.0 + t * t * top.0, u * u * by + 2.0 * u * t * ctrl.1 + t * t * top.1)
    }).collect();
    stroke(m, &trunk, height * 0.035, height * 0.016);
    // Rings on the trunk.
    for i in (3..38).step_by(3) {
        let (x, y) = trunk[i];
        let r = height * (0.035 - 0.019 * i as f32 / 40.0) + 2.0;
        disc(m, x, y, r);
    }
    let fronds = 9;
    for f in 0..fronds {
        let ang = -std::f32::consts::PI * (0.04 + 0.92 * f as f32 / (fronds - 1) as f32) + (rng.f() - 0.5) * 0.25;
        let len = height * (0.36 + rng.f() * 0.14);
        let droop = 0.9 + rng.f() * 0.6;
        let pts: Vec<(f32, f32)> = (0..=24).map(|i| {
            let t = i as f32 / 24.0;
            (top.0 + ang.cos() * len * t, top.1 + ang.sin() * len * t * 0.55 + droop * len * 0.5 * t * t)
        }).collect();
        stroke(m, &pts, height * 0.018, 1.5);
        // Leaflets hanging off both sides.
        for i in 2..23 {
            let (x, y) = pts[i];
            let (nx, ny) = (pts[i + 1].0 - pts[i - 1].0, pts[i + 1].1 - pts[i - 1].1);
            let l = (nx * nx + ny * ny).sqrt().max(0.01);
            let (px, py) = (-ny / l, nx / l);
            let leaf = height * 0.1 * (1.0 - i as f32 / 28.0);
            for side in [-1.0f32, 1.0] {
                let end = (x + px * leaf * side + nx / l * leaf * 0.4, y + py * leaf * side + leaf * 0.6);
                stroke(m, &[(x, y), end], 2.6, 0.8);
            }
        }
    }
    disc(m, top.0, top.1, height * 0.03);
}

impl Synthwave {
    pub fn new(w: i32, h: i32) -> Synthwave {
        let (w, h) = (w.max(16) as usize, h.max(16) as usize);
        let horizon = h * 60 / 100;
        // Sky: deep violet to hot pink at the horizon.
        let stops = [(0.0, hex(0x07021A)), (0.45, hex(0x26094A)), (0.8, hex(0x6B1466)), (1.0, hex(0xE0407B))];
        let mut sky = vec![0u32; w * horizon];
        for y in 0..horizon {
            let v = y as f32 / horizon as f32;
            let j = stops.iter().rposition(|s| s.0 <= v).unwrap_or(0).min(stops.len() - 2);
            let (a, b) = (stops[j], stops[j + 1]);
            let c = pack(mix3(a.1, b.1, (v - a.0) / (b.0 - a.0)));
            sky[y * w..(y + 1) * w].fill(c);
        }
        let mut rng = Rng(0x5EED_0042);
        let mut stars = Vec::new();
        for _ in 0..240 {
            let y = (rng.f().powf(1.6) * horizon as f32 * 0.62) as usize;
            let x = rng.below(w as u32) as usize;
            let b = 90 + rng.below(166);
            stars.push((y * w + x, b, rng.f() * std::f32::consts::TAU));
        }
        let sun_r = h as f32 * 0.24;
        // The sun's halo: a blurred disc, drawn as light.
        let hs = (sun_r * 3.2) as usize;
        let mut halo = Mask::new(hs, hs);
        disc(&mut halo, hs as f32 / 2.0, hs as f32 / 2.0, sun_r * 0.95);
        let halo = halo.blurred((sun_r * 0.28) as usize, 3);

        // Mountains on both sides, leaving the sun clear.
        let mh = (h as f32 * 0.2) as usize;
        let mut mm = Mask::new(w, mh);
        let mut rim = Mask::new(w, mh);
        let mut peaks = Vec::new();
        let mut rng = Rng(0xA11CE);
        let mut x = 0.0f32;
        while x < w as f32 {
            let side = ((x - w as f32 / 2.0).abs() / (w as f32 / 2.0)).clamp(0.0, 1.0);
            let tall = smoothstep(0.22, 0.95, side) * (0.55 + 0.45 * rng.f());
            peaks.push((x, mh as f32 * (1.0 - tall * 0.95)));
            x += 50.0 + rng.f() * 60.0;
        }
        peaks.push((w as f32, mh as f32 * 0.3));
        for xx in 0..w {
            let fx = xx as f32;
            let j = peaks.iter().rposition(|p| p.0 <= fx).unwrap_or(0).min(peaks.len() - 2);
            let (a, b) = (peaks[j], peaks[j + 1]);
            let top = a.1 + (b.1 - a.1) * (fx - a.0) / (b.0 - a.0).max(1.0);
            for y in 0..mh {
                let d = y as f32 - top;
                if d >= 0.0 {
                    mm.a[y * w + xx] = 255;
                }
                if (-1.0..2.0).contains(&d) && top < mh as f32 - 2.0 {
                    rim.a[y * w + xx] = 255;
                }
            }
        }
        let my = horizon as i32 - mh as i32;
        let mut mountains = Sprite::from(0, my, &mm, |_, y| pack(mix3(hex(0x1C0738), hex(0x2E0B4F), y as f32 / mh as f32)));
        let rim_glow = rim.padded(6).blurred(2, 2);
        let rim_glow = Sprite::from(-6, my - 6, &rim_glow, |_, _| PINK);
        // Merge the rim's glow into the mountain layer as light later.
        mountains.px.extend(Sprite::from(0, my, &rim, |_, _| 0xFF7AC8).px);

        // Palm trees: a pair on the left, one on the right.
        let mut pm = Mask::new(w, h);
        let s = h as f32 / 800.0;
        palm(&mut pm, 175.0 * s, h as f32 + 20.0, 560.0 * s, 70.0 * s, 7);
        palm(&mut pm, 60.0 * s, h as f32 + 20.0, 400.0 * s, -30.0 * s, 11);
        palm(&mut pm, w as f32 - 150.0 * s, h as f32 + 20.0, 500.0 * s, -80.0 * s, 23);
        let palms = Sprite::from(0, 0, &pm, |_, _| 0x0B0216);

        // Title: chrome letters (cyan sky above a dark line, pink and orange
        // below), with a pink neon glow.
        let sans = gfx::font_sans();
        let tm = gfx::text_mask(&sans, 150.0 * s, 4.0, crate::TITLE);
        let tx = (w as i32 - tm.w as i32) / 2;
        let ty = (h as f32 * 0.06) as i32;
        let th = tm.h as f32;
        let title = Sprite::from(tx, ty, &tm, |_, y| {
            let v = y as f32 / th;
            let c = if v < 0.52 {
                mix3(hex(0xF4FDFF), hex(0x39C6F0), v / 0.52)
            } else if v < 0.56 {
                hex(0x2A0B4A)
            } else {
                mix3(hex(0xFF2E97), hex(0xFFC46B), (v - 0.56) / 0.44)
            };
            pack(c)
        });
        let pad = 18;
        let tg = tm.padded(pad).blurred(7, 3);
        let title_glow = Sprite::from(tx - pad as i32, ty - pad as i32, &tg, |_, _| PINK);
        let sm = gfx::text_mask(&sans, 30.0 * s, 16.0, &format!("OS {}", crate::VERSION));
        let sx = (w as i32 - sm.w as i32) / 2;
        let sy = ty + tm.h as i32 + (20.0 * s) as i32;
        let sub = Sprite::from(sx, sy, &sm, |_, _| 0xD8FBFF);
        let sg = sm.padded(10).blurred(4, 3);
        let sub_glow = Sprite::from(sx - 10, sy - 10, &sg, |_, _| CYAN);

        mountains.px.shrink_to_fit();
        Synthwave { w, h, horizon, sky, stars, sun_r, halo, mountains, rim_glow, palms, title, title_glow, sub, sub_glow }
    }

    fn sun(&self, px: &mut [u32], t: f32) {
        let (w, hz) = (self.w, self.horizon);
        let rise = smoothstep(0.0, 1.7, t);
        let cy = hz as f32 + self.sun_r * 1.05 - rise * self.sun_r * 1.3;
        let cx = w as f32 / 2.0;
        // Halo as light, above the horizon only.
        let hs = self.halo.w as i32;
        let (hx, hy) = (cx as i32 - hs / 2, cy as i32 - hs / 2);
        for my in 0..self.halo.h {
            let y = hy + my as i32;
            if y < 0 || y as usize >= hz {
                continue;
            }
            for mx in 0..self.halo.w {
                let a = self.halo.a[my * self.halo.w + mx] as u32;
                let x = hx + mx as i32;
                if a > 2 && x >= 0 && (x as usize) < w {
                    let i = y as usize * w + x as usize;
                    px[i] = add(px[i], 0xFF3D7F, (a * 150) >> 8);
                }
            }
        }
        // The disc: yellow to pink, with gaps in its lower half that drift down.
        let r = self.sun_r;
        let top = cy - r;
        let drift = (t * 14.0) % 26.0;
        for y in (top.max(0.0) as usize)..((cy + r) as usize).min(hz) {
            let dy = y as f32 + 0.5 - cy;
            let half = (r * r - dy * dy).max(0.0).sqrt();
            let v = (y as f32 - top) / (2.0 * r);
            if v > 0.32 {
                let band = (y as f32 - top + 26.0 - drift) % 26.0;
                let gap = 1.5 + 13.0 * (v - 0.32) / 0.5;
                if band < gap {
                    continue;
                }
            }
            let c = if v < 0.5 { mix3(hex(0xFFF27A), hex(0xFF9A3C), v / 0.5) } else { mix3(hex(0xFF9A3C), hex(0xFF2E97), (v - 0.5) / 0.5) };
            let c = pack(c);
            let (x0, x1) = (cx - half, cx + half);
            let row = y * w;
            for x in (x0.max(0.0) as usize)..(x1.ceil() as usize).min(w) {
                let cov = ((x as f32 + 1.0 - x0).min(1.0)).min(x1 - x as f32).clamp(0.0, 1.0);
                px[row + x] = over(px[row + x], c, (cov * 256.0) as u32);
            }
        }
    }

    fn floor(&self, px: &mut [u32], t: f32) {
        let (w, h, hz) = (self.w, self.h, self.horizon);
        let depth = (h - hz) as f32;
        for y in hz..h {
            let v = (y - hz) as f32 / depth;
            let c = pack(mix3(hex(0x3A0D5C), hex(0x0A0218), v.powf(0.5)));
            px[y * w..(y + 1) * w].fill(c);
        }
        // Haze just under the horizon.
        for y in hz..(hz + 40).min(h) {
            let k = (1.0 - (y - hz) as f32 / 40.0).powi(2);
            for p in &mut px[y * w..(y + 1) * w] {
                *p = add(*p, PINK, (k * 120.0) as u32);
            }
        }
        // Horizontal lines, moving towards the viewer.
        let speed = 1.4 * (0.5 + 0.5 * smoothstep(0.0, 1.0, t));
        let phase = (t * speed).fract();
        for k in 1..20 {
            let z = k as f32 - phase;
            if z < 0.35 {
                continue;
            }
            let y = hz as f32 + depth * 0.45 / z;
            if y >= h as f32 + 3.0 {
                continue;
            }
            let near = ((y - hz as f32) / 110.0).clamp(0.0, 1.0).powf(1.6);
            for (dy, s) in [(0i32, 230u32), (-1, 110), (1, 110), (-2, 50), (2, 50), (-3, 20), (3, 20)] {
                let yy = y as i32 + dy;
                if yy <= hz as i32 || yy as usize >= h {
                    continue;
                }
                let kk = (s as f32 * near) as u32;
                let col = if dy == 0 { 0xFF7AC8 } else { PINK };
                for p in &mut px[yy as usize * w..(yy as usize + 1) * w] {
                    *p = add(*p, col, kk);
                }
            }
        }
        // Lines running to the vanishing point.
        let cx = w as f32 / 2.0;
        let spread = w as f32 * 0.16;
        for y in hz + 1..h {
            let d = (y - hz) as f32 / depth;
            let near = (d * 6.0).clamp(0.0, 1.0);
            for i in -24i32..=24 {
                let x = cx + i as f32 * spread * d;
                if x < -4.0 || x > w as f32 + 4.0 {
                    continue;
                }
                let xi = x.floor() as i32;
                let f = x - xi as f32;
                for (dx, s) in [(-3i32, 18.0f32), (-2, 45.0), (-1, 100.0), (0, 230.0), (1, 230.0), (2, 100.0), (3, 45.0), (4, 18.0)] {
                    let xx = xi + dx;
                    if xx < 0 || xx as usize >= w {
                        continue;
                    }
                    // Weight the two core pixels by where the line falls.
                    let s = match dx {
                        0 => s * (1.0 - f) + 100.0 * f,
                        1 => 100.0 * (1.0 - f) + s * f,
                        _ => s,
                    };
                    let i = y * w + xx as usize;
                    px[i] = add(px[i], if dx == 0 || dx == 1 { 0xFF7AC8 } else { PINK }, (s * near) as u32);
                }
            }
        }
        // The horizon line itself, bright.
        let row = hz * w;
        for p in &mut px[row..row + w] {
            *p = add(*p, 0xFFD0EC, 200);
        }
    }
}

/// A neon tube's flicker as it strikes: off and on a few times, then steady.
fn neon(t: f32, start: f32) -> u32 {
    let d = t - start;
    if d < 0.0 {
        return 0;
    }
    const STEPS: [(f32, u32); 8] = [(0.05, 200), (0.1, 0), (0.16, 256), (0.22, 40), (0.3, 256), (0.34, 90), (0.42, 256), (9e9, 256)];
    STEPS.iter().find(|s| d < s.0).map(|s| s.1).unwrap_or(256)
}

impl Scene for Synthwave {
    fn length(&self) -> f32 {
        3.2
    }

    fn draw(&mut self, t: f32, out: &mut SoftCanvas) {
        let w = self.w;
        let px = &mut out.px;
        let hz = self.horizon;
        px[..w * hz].copy_from_slice(&self.sky);
        for &(i, b, ph) in &self.stars {
            let tw = 0.6 + 0.4 * (t * 2.3 + ph).sin();
            px[i] = add(px[i], 0xFFFFFF, (b as f32 * tw) as u32);
        }
        self.sun(px, t);
        self.mountains.draw(px, w, 256, false);
        self.rim_glow.draw(px, w, 200, true);
        self.floor(px, t);
        self.palms.draw(px, w, 256, false);
        let k = neon(t, 1.5);
        if k > 0 {
            self.title_glow.draw(px, w, (k * 250) >> 8, true);
            self.title.draw(px, w, k, false);
        }
        let k = neon(t, 2.2);
        if k > 0 {
            self.sub_glow.draw(px, w, (k * 300) >> 8, true);
            self.sub.draw(px, w, k, false);
        }
    }
}
