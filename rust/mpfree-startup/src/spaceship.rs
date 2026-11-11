//! The spaceship theme's startup screen: a ship's bridge drops out of warp
//! in front of a purple spiral galaxy while its HUD boots around the view.
//!
//! - 0.0 s: stars streak past at warp; the window's frame draws itself.
//! - 1.35 s: the ship drops out of warp in a violet flash and the galaxy
//!   fades in, turning slowly (inner stars faster than outer ones).
//! - 2.0 s: the HUD comes up: a heading tape, a reticle round the core,
//!   readouts typing on the left, level bars on the right.
//! - 2.6 s: "MPFree" is wiped on by a scan line, then "OS v0.1".
//!
//! Made once: the star field with the galaxy's glow (two copies, with and
//! without it, to fade between), ~11,000 galaxy particles, and the text.
//! A frame copies the background, moves and adds the particles (a few
//! pixels each) and draws the HUD's thin lines.

use crate::gfx::{self, add, hex, light_line, mix3, pack, smoothstep, Mask, Rng, Sprite};
use crate::Scene;
use mpfree_ui::SoftCanvas;
use std::f32::consts::{PI, TAU};

const LILAC: u32 = 0xA78BFA;
const WHITE: u32 = 0xF2EEFF;

struct Particle {
    r: f32,
    theta: f32,
    omega: f32,
    color: u32,
    k: u32,
}

pub struct Spaceship {
    w: usize,
    h: usize,
    space: Vec<u32>,
    lit: Vec<u32>,
    center: (f32, f32),
    particles: Vec<Particle>,
    warp: Vec<(f32, f32, f32)>,
    title: Sprite,
    title_glow: Sprite,
    sub: Sprite,
    readouts: Vec<Sprite>,
    ticks: Vec<(i32, Sprite)>,
}

const TILT: f32 = 0.36;
const ROLL: f32 = -0.3;

impl Spaceship {
    pub fn new(w: i32, h: i32) -> Spaceship {
        let (w, h) = (w.max(16) as usize, h.max(16) as usize);
        let s = h as f32 / 800.0;
        let center = (w as f32 / 2.0, h as f32 * 0.4);
        let radius = h as f32 * 0.42;
        let mut rng = Rng(0x0BA1_7E57);

        // Space: black with a faint violet haze and a vignette, and stars.
        let mut space = vec![0u32; w * h];
        let (bw, bh) = (w / 8, h / 8);
        let mut haze = Mask::new(bw, bh);
        for _ in 0..26 {
            let (x, y) = (rng.below(bw as u32) as f32, rng.below(bh as u32) as f32);
            let r = 4.0 + rng.f() * 14.0;
            for yy in 0..bh {
                for xx in 0..bw {
                    let d = ((xx as f32 - x).powi(2) + (yy as f32 - y).powi(2)).sqrt();
                    if d < r {
                        let i = yy * bw + xx;
                        haze.a[i] = haze.a[i].saturating_add(((1.0 - d / r) * 22.0) as u8);
                    }
                }
            }
        }
        let haze = haze.blurred(4, 3);
        for y in 0..h {
            for x in 0..w {
                let hv = haze.at((x / 8).min(bw - 1), (y / 8).min(bh - 1)) as f32;
                let (dx, dy) = (x as f32 / w as f32 - 0.5, y as f32 / h as f32 - 0.45);
                let vig = (1.0 - (dx * dx + dy * dy) * 1.6).clamp(0.0, 1.0);
                let c = mix3(hex(0x030208), hex(0x2A1450), hv / 80.0);
                space[y * w + x] = pack([c[0] * vig, c[1] * vig, c[2] * vig]);
            }
        }
        for _ in 0..1100 {
            let i = rng.below((w * h) as u32) as usize;
            let b = rng.f().powi(3);
            let c = if rng.below(4) == 0 { LILAC } else { 0xFFFFFF };
            space[i] = add(space[i], c, (40.0 + 216.0 * b) as u32);
            if b > 0.7 && i + 1 < space.len() && i > w {
                space[i + 1] = add(space[i + 1], c, 60);
                space[i - 1] = add(space[i - 1], c, 60);
                space[i - w] = add(space[i - w], c, 60);
                if i + w < space.len() {
                    space[i + w] = add(space[i + w], c, 60);
                }
            }
        }

        // The galaxy's glow, flattened and rolled like the disc: a soft
        // violet disc and a white core.
        let mut lit = space.clone();
        let (cr, sr) = (ROLL.cos(), ROLL.sin());
        let gx0 = (center.0 - radius * 1.3).max(0.0) as usize;
        let gx1 = ((center.0 + radius * 1.3) as usize).min(w);
        let gy0 = (center.1 - radius * 0.9).max(0.0) as usize;
        let gy1 = ((center.1 + radius * 0.9) as usize).min(h);
        for y in gy0..gy1 {
            for x in gx0..gx1 {
                let (dx, dy) = (x as f32 - center.0, y as f32 - center.1);
                // Into the disc's own plane.
                let (u, v) = (dx * cr + dy * sr, -dx * sr + dy * cr);
                let v = v / TILT;
                let d = (u * u + v * v).sqrt() / radius;
                let disc = (-d * 3.2).exp() * 0.75;
                let core = (-d * d * 260.0).exp() * 1.4 + (-d * 14.0).exp() * 0.9;
                let i = y * w + x;
                lit[i] = add(lit[i], 0x6A3FD8, (disc * 300.0) as u32);
                lit[i] = add(lit[i], 0xFFF4FF, (core * 255.0) as u32);
            }
        }

        // Particles in two main arms, two faint ones, a bulge and a halo.
        let mut particles = Vec::new();
        for i in 0..11000 {
            let kind = rng.f();
            let (rn, theta) = if kind < 0.18 {
                let rn = (rng.f() * rng.f()) * 0.22;
                (rn, rng.f() * TAU)
            } else if kind < 0.93 {
                let arm = (i % 4) as f32;
                let rn = 0.06 + rng.f().powf(0.8) * 0.94;
                let wind = 2.0 * (1.0 + rn * 10.0).ln();
                let scatter = (rng.f() - 0.5 + rng.f() - 0.5) * (0.5 - 0.25 * rn);
                (rn, arm * PI / 2.0 + wind + scatter)
            } else {
                (rng.f().sqrt() * 1.05, rng.f() * TAU)
            };
            let major = i % 2 == 0 || kind < 0.18;
            let bright = (-rn * 2.2).exp() * if major { 1.0 } else { 0.55 };
            let color = if rn < 0.12 {
                pack(mix3(hex(0xFFFFFF), hex(0xE4D8FF), rn / 0.12))
            } else if rng.below(23) == 0 {
                0xFF7AD9
            } else if rng.below(3) == 0 {
                0xFFFFFF
            } else {
                pack(mix3(hex(0xB9A2FF), hex(0x6A5BFF), rn))
            };
            let k = ((60.0 + 200.0 * bright) * (0.6 + 0.4 * rng.f())) as u32;
            particles.push(Particle { r: rn * radius, theta, omega: 0.16 / (0.3 + rn * 2.0), color, k });
        }

        // Warp streaks: (angle, start distance, speed).
        let warp = (0..520).map(|_| (rng.f() * TAU, 10.0 + rng.f() * 400.0, 0.6 + rng.f() * 0.8)).collect();

        // Title and text.
        let sans = gfx::font_sans();
        let mono = gfx::font_mono();
        let tm = gfx::text_mask(&sans, 92.0 * s, 26.0 * s, crate::TITLE);
        let tx = (w as i32 - tm.w as i32) / 2;
        let ty = (h as f32 * 0.72) as i32;
        let title = Sprite::from(tx, ty, &tm, |_, _| 0xFFFFFF);
        let tg = tm.padded(16).blurred(6, 3);
        let title_glow = Sprite::from(tx - 16, ty - 16, &tg, |_, _| 0x8B5CF6);
        let sm = gfx::text_mask(&mono, 20.0 * s, 10.0 * s, &format!("OS {}  //  SYSTEMS ONLINE", crate::VERSION));
        let sub = Sprite::from((w as i32 - sm.w as i32) / 2, ty + tm.h as i32 + (22.0 * s) as i32, &sm, |_, _| LILAC);
        let lines = ["NAV      LOCKED", "SECTOR   MPC-1", "DRIVE    IDLE", "COMMS    QUIET"];
        let readouts = lines.iter().enumerate().map(|(i, l)| {
            let m = gfx::text_mask(&mono, 15.0 * s, 1.5, l);
            Sprite::from((60.0 * s) as i32, (h as f32 * 0.2 + i as f32 * 28.0 * s) as i32, &m, |_, _| 0xC9B8FF)
        }).collect();
        let ticks = (0..36).map(|i| {
            let m = gfx::text_mask(&mono, 12.0 * s, 1.0, &format!("{:03}", i * 10));
            (i * 10, Sprite::from(0, 0, &m, |_, _| 0x9D8CD6))
        }).collect();
        Spaceship { w, h, space, lit, center, particles, warp, title, title_glow, sub, readouts, ticks }
    }

    /// How far along the warp is, by distance travelled (eases in, then
    /// stops dead at 1.35 s).
    fn warp_travel(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.35);
        t * t * 1.4
    }

    fn frame_path(&self) -> Vec<(f32, f32)> {
        let s = self.h as f32 / 800.0;
        let (m, cut) = (22.0 * s, 46.0 * s);
        let (w, h) = (self.w as f32 - 1.0, self.h as f32 - 1.0);
        vec![(m + cut, m), (w - m - cut, m), (w - m, m + cut), (w - m, h - m - cut), (w - m - cut, h - m),
             (m + cut, h - m), (m, h - m - cut), (m, m + cut), (m + cut, m)]
    }

    fn hud(&self, px: &mut [u32], t: f32) {
        let (w, h) = (self.w, self.h);
        let s = h as f32 / 800.0;
        // The window frame draws itself round, then stays.
        let path = self.frame_path();
        let total: f32 = path.windows(2).map(|p| ((p[1].0 - p[0].0).powi(2) + (p[1].1 - p[0].1).powi(2)).sqrt()).sum();
        let mut left = total * smoothstep(0.1, 0.9, t);
        for seg in path.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            if left <= 0.0 {
                break;
            }
            let f = (left / len).min(1.0);
            let end = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            light_line(px, w, a, end, LILAC, 150, 150);
            left -= len;
        }
        let k = smoothstep(2.0, 2.4, t);
        if k <= 0.0 {
            return;
        }
        let kk = (k * 256.0) as u32;
        // Heading tape along the top, drifting slowly.
        let (y0, x0, x1) = (52.0 * s, w as f32 * 0.3, w as f32 * 0.7);
        light_line(px, w, (x0, y0), (x1, y0), LILAC, (90 * kk) >> 8, (90 * kk) >> 8);
        let heading = 172.0 + (t - 2.0).max(0.0) * 1.5;
        let spacing = 8.0 * s;
        let first = (heading - (x1 - x0) / spacing / 2.0).floor() as i32;
        for d in first..first + ((x1 - x0) / spacing) as i32 + 2 {
            let x = (w as f32 / 2.0) + (d as f32 - heading) * spacing;
            if x < x0 || x > x1 {
                continue;
            }
            let fade = 1.0 - ((x - w as f32 / 2.0).abs() / ((x1 - x0) / 2.0)).powi(2);
            let major = d.rem_euclid(10) == 0;
            let len = if major { 12.0 * s } else { 5.0 * s };
            let a = (fade * if major { 220.0 } else { 120.0 } * k) as u32;
            light_line(px, w, (x, y0), (x, y0 + len), LILAC, a, a);
            if major {
                if let Some((_, spr)) = self.ticks.iter().find(|(v, _)| *v == d.rem_euclid(360)) {
                    spr.draw_at(px, w, x as i32 - spr.w as i32 / 2, (y0 + 16.0 * s) as i32, a.min(256));
                }
            }
        }
        // The centre mark of the tape.
        let cx = w as f32 / 2.0;
        light_line(px, w, (cx - 6.0 * s, y0 - 10.0 * s), (cx, y0 - 2.0 * s), WHITE, (220 * kk) >> 8, (220 * kk) >> 8);
        light_line(px, w, (cx + 6.0 * s, y0 - 10.0 * s), (cx, y0 - 2.0 * s), WHITE, (220 * kk) >> 8, (220 * kk) >> 8);
        // A reticle round the core: four corners, turning.
        let (gx, gy) = self.center;
        let rr = 64.0 * s * (1.0 + 0.6 * (1.0 - smoothstep(2.0, 2.6, t)));
        let spin = t * 0.25;
        for q in 0..4 {
            let a = spin + q as f32 * PI / 2.0 + PI / 4.0;
            let (ca, sa) = (a.cos(), a.sin());
            let p = (gx + ca * rr, gy + sa * rr);
            let arm = 14.0 * s;
            let (t1, t2) = ((-sa, ca), (-ca, -sa));
            let kk2 = (200 * kk) >> 8;
            light_line(px, w, p, (p.0 + t1.0 * arm, p.1 + t1.1 * arm), WHITE, kk2, kk2);
            light_line(px, w, p, (p.0 - t1.0 * arm, p.1 - t1.1 * arm), WHITE, kk2, kk2);
            light_line(px, w, p, (p.0 + t2.0 * arm * 0.7, p.1 + t2.1 * arm * 0.7), WHITE, kk2, kk2);
        }
        // Readouts typing on the left.
        for (i, r) in self.readouts.iter().enumerate() {
            let start = 2.3 + i as f32 * 0.18;
            if t > start {
                let upto = ((t - start) * 900.0 * s) as i32;
                r.draw_upto(px, w, 256, true, upto);
            }
        }
        // Level bars on the right, breathing.
        let bx = w as f32 - 60.0 * s;
        for i in 0..10 {
            let y = h as f32 * 0.2 + i as f32 * 11.0 * s;
            let level = 0.35 + 0.65 * (0.5 + 0.5 * (t * 1.3 + i as f32 * 0.7).sin()) * k;
            let len = 90.0 * s * level;
            for dy in 0..(4.0 * s) as i32 {
                let yy = y + dy as f32;
                light_line(px, w, (bx - len, yy), (bx, yy), if i == 0 { WHITE } else { LILAC }, (60 * kk) >> 8, (170 * kk) >> 8);
            }
        }
    }
}

impl Scene for Spaceship {
    fn length(&self) -> f32 {
        3.6
    }

    fn draw(&mut self, t: f32, out: &mut SoftCanvas) {
        let (w, h) = (self.w, self.h);
        let px = &mut out.px;
        // Background, fading the galaxy's glow in after the warp.
        let fade = smoothstep(1.35, 2.3, t);
        if fade >= 1.0 {
            px.copy_from_slice(&self.lit);
        } else if fade <= 0.0 {
            px.copy_from_slice(&self.space);
        } else {
            let k = (fade * 256.0) as u32;
            for ((p, &a), &b) in px.iter_mut().zip(&self.space).zip(&self.lit) {
                *p = gfx::over(a, b, k);
            }
        }

        // Warp streaks.
        if t < 1.5 {
            let (cx, cy) = self.center;
            let d = Self::warp_travel(t);
            let prev = Self::warp_travel(t - 0.05);
            let k = 1.0 - smoothstep(1.3, 1.5, t);
            for &(a, r0, sp) in &self.warp {
                let r1 = r0 * (1.0 + d * sp * 3.0);
                let r2 = r0 * (1.0 + prev * sp * 3.0);
                let (ca, sa) = (a.cos(), a.sin());
                let near = (r1 / 600.0).min(1.0);
                let kk = (k * 255.0 * (0.25 + near)) as u32;
                light_line(px, w, (cx + ca * r2, cy + sa * r2), (cx + ca * r1, cy + sa * r1),
                           if r0 < 120.0 { WHITE } else { LILAC }, kk / 5, kk.min(256));
            }
        }

        // The galaxy, turning.
        if fade > 0.0 {
            let (cx, cy) = self.center;
            let (cr, sr) = (ROLL.cos(), ROLL.sin());
            let kf = (fade * 256.0) as u32;
            let tt = t - 1.35;
            for p in &self.particles {
                let a = p.theta + p.omega * tt;
                let (u, v) = (a.cos() * p.r, a.sin() * p.r * TILT);
                let x = cx + u * cr - v * sr;
                let y = cy + u * sr + v * cr;
                if x < 1.0 || y < 1.0 || x >= (w - 2) as f32 || y >= (h - 2) as f32 {
                    continue;
                }
                let i = y as usize * w + x as usize;
                let k = (p.k * kf) >> 8;
                px[i] = add(px[i], p.color, k);
                if p.k > 150 {
                    let k2 = k / 3;
                    px[i + 1] = add(px[i + 1], p.color, k2);
                    px[i + w] = add(px[i + w], p.color, k2);
                }
            }
        }

        // Drop out of warp: a violet flash.
        if (1.3..1.9).contains(&t) {
            let k = (1.0 - smoothstep(1.35, 1.9, t)) * smoothstep(1.3, 1.36, t);
            let kk = (k * 150.0) as u32;
            for p in px.iter_mut() {
                *p = add(*p, 0xC4B0FF, kk);
            }
        }

        self.hud(px, t);

        // Title: wiped on by a scan line, then the subtitle.
        if t > 2.6 {
            let wipe = smoothstep(2.6, 3.2, t);
            let upto = (wipe * (self.title_glow.w as f32 + 40.0)) as i32;
            self.title_glow.draw_upto(px, w, 230, true, upto);
            self.title.draw_upto(px, w, 256, false, upto - 16);
            if wipe < 1.0 {
                let x = (self.title_glow.x + upto) as f32;
                let (y0, y1) = (self.title.y as f32 - 20.0, self.title.y as f32 + 120.0);
                for dx in -2i32..=2 {
                    let k = [60, 140, 256, 140, 60][(dx + 2) as usize];
                    light_line(px, w, (x + dx as f32, y0), (x + dx as f32, y1), WHITE, k, k);
                }
            }
        }
        if t > 3.1 {
            let upto = ((t - 3.1) * 1400.0) as i32;
            self.sub.draw_upto(px, w, 256, true, upto);
        }
    }
}
