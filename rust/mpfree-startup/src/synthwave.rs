//! The synthwave theme's startup screen: a striped sun rising over rolling
//! neon wireframe hills, with "MPFree" slashed across the sun in a brush
//! marker face.
//!
//! Made once: the sky with its stars, the sun's halo, and the title layers
//! (outline, glow, the letters with their pink-to-yellow fill and slash
//! cuts, the streaks beside them, the subtitle). A frame copies the sky,
//! places the sun, and draws the hills: rows of a height field from near to
//! far, each row filling only what nothing nearer already covers (a
//! "floating horizon"), with glowing lines along and across the rows.
//!
//! - 0.0 s: the sun rises behind the hills while they start rolling in.
//! - 1.3 s: the title is slashed on from the left, then its glow strikes.
//! - 2.0 s: the subtitle flickers on.
//! - 3.0 s: settled; the hills keep rolling and the stars twinkle.

use crate::gfx::{self, add, hex, mix3, over, pack, smoothstep, Mask, Rng, Sprite};
use crate::Scene;
use mpfree_ui::SoftCanvas;

const LINE: u32 = 0xFF2BD6;
const CYAN: u32 = 0x37E8FF;

pub struct Synthwave {
    w: usize,
    h: usize,
    horizon: usize,
    sky: Vec<u32>,
    stars: Vec<(usize, u32, f32)>,
    sun_r: f32,
    halo: Mask,
    hor: Vec<f32>,
    title_glow: Sprite,
    title_edge: Sprite,
    title: Sprite,
    streaks: Sprite,
    sub: Sprite,
    sub_edge: Sprite,
    sub_glow: Sprite,
}

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

/// The hills' height at world (x, z): a flat valley in the middle for the
/// sun, rolling hills to the sides.
fn height(x: f32, z: f32) -> f32 {
    let side = smoothstep(1.2, 6.0, x.abs());
    let roll = 0.55 * (0.45 * x + 0.38 * z).sin() + 0.35 * (1.1 * x - 0.61 * z + 1.3).sin() + 0.2 * (0.23 * z + 0.4 * x).cos();
    let swell = 0.12 * (0.5 * z).sin() * (0.7 * x).cos();
    side * (0.6 + roll) * 1.5 + swell
}

impl Synthwave {
    pub fn new(w: i32, h: i32) -> Synthwave {
        let (w, h) = (w.max(16) as usize, h.max(16) as usize);
        let s = h as f32 / 800.0;
        let horizon = h * 62 / 100;
        let stops = [(0.0, hex(0x05021A)), (0.5, hex(0x1C0840)), (0.85, hex(0x4A1260)), (1.0, hex(0xB0306F))];
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
        for _ in 0..260 {
            let y = (rng.f().powf(1.4) * horizon as f32 * 0.7) as usize;
            let x = rng.below(w as u32) as usize;
            stars.push((y * w + x, 80 + rng.below(176), rng.f() * std::f32::consts::TAU));
        }
        let sun_r = h as f32 * 0.26;
        let hs = (sun_r * 3.2) as usize;
        let mut halo = Mask::new(hs, hs);
        disc(&mut halo, hs as f32 / 2.0, hs as f32 / 2.0, sun_r * 0.95);
        let halo = halo.blurred((sun_r * 0.28) as usize, 3);

        // The title: Permanent Marker, leaning forward, with slash cuts
        // through its lower half.
        let marker = gfx::font_marker();
        let raw = gfx::text_mask(&marker, 210.0 * s, 2.0, crate::TITLE).padded(8);
        let lean = 0.24;
        let extra = (raw.h as f32 * lean).ceil() as usize;
        let mut tm = Mask::new(raw.w + extra, raw.h);
        for y in 0..raw.h {
            let shift = ((raw.h - y) as f32 * lean) as usize;
            for x in 0..raw.w {
                tm.a[y * tm.w + x + shift] = raw.at(x, y);
            }
        }
        let th = tm.h as f32;
        let cuts: Vec<(f32, f32)> = vec![(0.58, 0.022), (0.68, 0.03), (0.77, 0.036), (0.86, 0.04)];
        for y in 0..tm.h {
            let v = y as f32 / th;
            if cuts.iter().any(|&(at, wd)| (v - at).abs() < wd / 2.0) {
                tm.a[y * tm.w..(y + 1) * tm.w].fill(0);
            }
        }
        let tx = (w as i32 - tm.w as i32) / 2;
        let ty = (horizon as f32 - sun_r * 0.45 - th * 0.95) as i32;
        let title = Sprite::from(tx, ty, &tm, |_, y| {
            let v = y as f32 / th;
            let c = if v < 0.5 { mix3(hex(0xFF2EB4), hex(0xFF6A3D), v / 0.5) } else { mix3(hex(0xFF6A3D), hex(0xFFE45C), (v - 0.5) / 0.5) };
            pack(c)
        });
        let pad = 24;
        let edge = tm.padded(pad).dilated(4);
        let title_edge = Sprite::from(tx - pad as i32, ty - pad as i32, &edge, |_, _| 0x1A0430);
        let glow = edge.blurred(9, 3);
        let title_glow = Sprite::from(tx - pad as i32, ty - pad as i32, &glow, |_, _| 0xFF2E97);
        // Streaks: thin lines running out of the cuts to both sides.
        let mut sm = Mask::new(w, tm.h);
        let cx = w as f32 / 2.0;
        for &(at, wd) in &cuts {
            let y0 = ((at - wd / 2.0) * th) as usize;
            let y1 = (((at + wd / 2.0) * th) as usize).min(tm.h);
            let reach = w as f32 * (0.36 + at * 0.12);
            for y in y0..y1 {
                for x in 0..w {
                    let d = (x as f32 - cx).abs() / reach;
                    if d < 1.0 {
                        sm.a[y * w + x] = ((1.0 - d).powf(0.6) * 200.0) as u8;
                    }
                }
            }
        }
        let streaks = Sprite::from(0, ty, &sm, |_, y| pack(mix3(hex(0xFF2EB4), hex(0xFFB03D), y as f32 / th)));

        let sans = gfx::font_sans();
        let sub_m = gfx::text_mask(&sans, 40.0 * s, 14.0 * s, &format!("OS {}", crate::VERSION));
        let sx = (w as i32 - sub_m.w as i32) / 2;
        let sy = ty + tm.h as i32 + (6.0 * s) as i32;
        let sh = sub_m.h as f32;
        let sub = Sprite::from(sx, sy, &sub_m, |_, y| pack(mix3(hex(0xE8FDFF), hex(0x37E8FF), y as f32 / sh)));
        let sub_edge = Sprite::from(sx - 12, sy - 12, &sub_m.padded(12).dilated(3), |_, _| 0x12052A);
        let sg = sub_m.padded(12).blurred(5, 3);
        let sub_glow = Sprite::from(sx - 12, sy - 12, &sg, |_, _| CYAN);
        Synthwave { w, h, horizon, sky, stars, sun_r, halo, hor: vec![0.0; w], title_glow, title_edge, title, streaks, sub, sub_edge, sub_glow }
    }

    fn sun(&self, px: &mut [u32], t: f32) {
        let (w, hz) = (self.w, self.horizon);
        let rise = smoothstep(0.0, 1.6, t);
        let cy = hz as f32 + self.sun_r * 1.05 - rise * self.sun_r * 1.5;
        let cx = w as f32 / 2.0;
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
                    px[i] = add(px[i], 0xFF3D7F, (a * 140) >> 8);
                }
            }
        }
        let r = self.sun_r;
        let top = cy - r;
        let drift = (t * 14.0) % 26.0;
        for y in (top.max(0.0) as usize)..((cy + r) as usize).min(hz) {
            let dy = y as f32 + 0.5 - cy;
            let half = (r * r - dy * dy).max(0.0).sqrt();
            let v = (y as f32 - top) / (2.0 * r);
            if v > 0.4 {
                let band = (y as f32 - top + 26.0 - drift) % 26.0;
                if band < 1.5 + 13.0 * (v - 0.4) / 0.5 {
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

    /// The hills, near to far. `hor[x]` is the highest point drawn so far
    /// in each column: a farther row shows only above it.
    fn hills(&mut self, px: &mut [u32], t: f32) {
        let (w, h, hz) = (self.w, self.h, self.horizon as f32);
        let depth = h as f32 - hz;
        let cx = w as f32 / 2.0;
        let cam = 1.0;
        let f = depth / cam;
        let speed = 1.6 * (0.5 + 0.5 * smoothstep(0.0, 1.2, t));
        let scroll = t * speed;
        let phase = scroll.fract();
        const XS: i32 = 18;
        let project = |xw: f32, z: f32| -> (f32, f32) {
            let zw = z + scroll;
            (cx + xw * f / z, hz + (cam - height(xw, zw) * 1.15) * f / z)
        };
        self.hor.iter_mut().for_each(|v| *v = h as f32);
        let mut prev: Option<Vec<(f32, f32)>> = None;
        let mut k = 0;
        loop {
            let z = 0.55 + k as f32 * 0.5 - phase * 0.5;
            k += 1;
            if z > 34.0 {
                break;
            }
            let pts: Vec<(f32, f32)> = (-XS..=XS).map(|i| project(i as f32, z)).collect();
            let fog = 1.0 - smoothstep(6.0, 34.0, z);
            let near = 1.0 + (2.0 - z).max(0.0);
            let floor = pack(mix3(hex(0x2A0A45), hex(0x0D0322), (1.0 / z).min(1.0)));
            let old = self.hor.clone();
            // Fill the newly seen surface and draw the row's line.
            for seg in pts.windows(2) {
                let ((x0, y0), (x1, y1)) = (seg[0], seg[1]);
                if x1 < 0.0 || x0 >= w as f32 || x1 <= x0 {
                    continue;
                }
                let xa = x0.max(0.0) as usize;
                let xb = (x1.ceil() as usize).min(w);
                for x in xa..xb {
                    let u = ((x as f32 - x0) / (x1 - x0)).clamp(0.0, 1.0);
                    let y = y0 + (y1 - y0) * u;
                    if y >= self.hor[x] {
                        continue;
                    }
                    let yi = y.max(0.0) as usize;
                    let ye = (self.hor[x] as usize).min(h);
                    for yy in yi..ye {
                        px[yy * w + x] = floor;
                    }
                    let kk = 255.0 * fog;
                    for (dy, s) in [(0i32, 1.0f32), (-1, 0.45), (1, 0.45), (-2, 0.16), (2, 0.16)] {
                        let yy = yi as i32 + dy;
                        if yy >= 0 && (yy as usize) < h && (yy as f32) < old[x] + 1.0 {
                            let i = yy as usize * w + x;
                            px[i] = add(px[i], if dy == 0 { 0xFF8AE8 } else { LINE }, (kk * s * near.min(1.6)) as u32);
                        }
                    }
                    self.hor[x] = y;
                }
            }
            // Lines across the rows, from this row to the nearer one.
            if let Some(p) = &prev {
                for (a, b) in p.iter().zip(&pts) {
                    let n = ((b.1 - a.1).abs().max((b.0 - a.0).abs())).ceil().max(1.0) as i32;
                    for j in 0..=n {
                        let u = j as f32 / n as f32;
                        let (x, y) = (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
                        if x < 1.0 || x >= (w - 2) as f32 || y < 0.0 || y >= h as f32 {
                            continue;
                        }
                        let xi = x as usize;
                        if y > old[xi] + 1.0 {
                            continue;
                        }
                        let i = y as usize * w + xi;
                        let kk = 230.0 * fog;
                        px[i] = add(px[i], 0xFF8AE8, kk as u32);
                        px[i - 1] = add(px[i - 1], LINE, (kk * 0.4) as u32);
                        px[i + 1] = add(px[i + 1], LINE, (kk * 0.4) as u32);
                    }
                }
            }
            prev = Some(pts);
        }
        // Haze along the horizon.
        let hzi = hz as usize;
        for y in hzi.saturating_sub(10)..(hzi + 14).min(h) {
            let k = 1.0 - (y as f32 - hz).abs() / 14.0;
            for x in 0..w {
                // Only in columns where no hill stands above the horizon.
                if self.hor[x] >= hz - 2.0 {
                    let p = &mut px[y * w + x];
                    *p = add(*p, 0xFF4FA8, (k.max(0.0) * 70.0) as u32);
                }
            }
        }
    }
}

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
        3.0
    }

    fn draw(&mut self, t: f32, out: &mut SoftCanvas) {
        let w = self.w;
        let mut px = std::mem::take(&mut out.px);
        let hz = self.horizon;
        px[..w * hz].copy_from_slice(&self.sky);
        px[w * hz..].fill(0x0D0322);
        for &(i, b, ph) in &self.stars {
            let tw = 0.6 + 0.4 * (t * 2.3 + ph).sin();
            px[i] = add(px[i], 0xFFFFFF, (b as f32 * tw) as u32);
        }
        self.sun(&mut px, t);
        self.hills(&mut px, t);
        // The title: slashed on from the left with a hot edge, then its glow strikes.
        if t > 1.3 {
            let wipe = smoothstep(1.3, 1.65, t);
            let span = self.title_glow.w as f32 + 80.0;
            let upto = (wipe * span) as i32;
            let g = neon(t, 1.6);
            self.streaks.draw_upto(&mut px, w, (wipe * 200.0) as u32, true, upto + self.title_glow.x);
            if g > 0 {
                self.title_glow.draw(&mut px, w, (g * 230) >> 8, true);
            }
            self.title_edge.draw_upto(&mut px, w, 256, false, upto);
            self.title.draw_upto(&mut px, w, 256, false, upto - 24);
            if wipe < 1.0 {
                let x = (self.title_glow.x + upto) as f32;
                let (y0, y1) = (self.title.y as f32 - 10.0, self.title.y as f32 + 220.0);
                for dx in -3i32..=3 {
                    let k = [40, 90, 180, 256, 180, 90, 40][(dx + 3) as usize];
                    gfx::light_line(&mut px, w, (x + dx as f32 + 40.0, y0), (x + dx as f32 - 30.0, y1), 0xFFF0FA, k, k);
                }
            }
        }
        let k = neon(t, 2.0);
        if k > 0 {
            self.sub_edge.draw(&mut px, w, (k * 200) >> 8, false);
            self.sub_glow.draw(&mut px, w, (k * 300) >> 8, true);
            self.sub.draw(&mut px, w, k, false);
        }
        out.px = px;
    }
}
