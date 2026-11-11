//! The spaceship theme's startup screen: the view from a ship's bridge, a
//! purple spiral galaxy through the windshield while the controls boot.
//!
//! - 0.0 s: the galaxy fades up out of the dark, turning slowly (inner stars
//!   faster than outer ones); the bridge is a dark frame round it.
//! - 0.5 s: a light runs out along the dashboard's rim, the LED rows and
//!   sliders come on from the middle, and three screens power up: a radar
//!   sweep, a boot log with a progress bar, a spectrum with a waveform.
//! - 1.7 s: the windshield HUD comes up: corner brackets, a heading tape, a
//!   reticle that closes on the galaxy's core with a target label, readouts.
//! - 2.6 s: "MPFree" is wiped onto the glass by a scan line, then "OS v0.1";
//!   the centre screen reads SYSTEMS ONLINE.
//!
//! Made once: the star field and the galaxy's glow (with and without it, to
//! fade between), the bridge with its bezels and sockets drawn over both,
//! ~11,000 galaxy particles and the text. A frame copies the background,
//! adds the particles that are not behind the bridge, and draws the lights,
//! the screens' contents and the HUD, which are all small.

use crate::gfx::{self, add, hash, hex, light_line, mix3, over, pack, scale, smoothstep, Mask, Rng, Sprite};
use crate::Scene;
use mpfree_ui::SoftCanvas;
use std::f32::consts::{PI, TAU};

const LILAC: u32 = 0xA78BFA;
const VIOLET: u32 = 0x8B5CF6;
const WHITE: u32 = 0xF2EEFF;
const PINK: u32 = 0xFF7AD9;
const SOFT: u32 = 0xC9B8FF;

const TILT: f32 = 0.36;
const ROLL: f32 = -0.3;

struct Particle {
    r: f32,
    theta: f32,
    omega: f32,
    color: u32,
    k: u32,
}

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn frac(sw: f32, sh: f32, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x: (sw * x) as i32, y: (sh * y) as i32, w: (sw * w).max(4.0) as i32, h: (sh * h).max(4.0) as i32 }
    }
}

struct Led {
    x: i32,
    y: i32,
    color: u32,
    on: f32,
}

/// The bridge's outline: where the glass ends.
struct Shape {
    w: f32,
    h: f32,
}

impl Shape {
    fn dx(&self, x: f32) -> f32 {
        (x - self.w / 2.0) / (self.w / 2.0)
    }
    /// The bottom edge of the canopy frame.
    fn top(&self, x: f32) -> f32 {
        self.h * (0.05 + 0.035 * self.dx(x).powi(2))
    }
    /// The dashboard's rim, higher in the middle.
    fn dash(&self, x: f32) -> f32 {
        self.h * (0.70 + 0.07 * self.dx(x).powi(2))
    }
    /// The inner edge of the left pillar (the right one mirrors it).
    fn pillar(&self, y: f32) -> f32 {
        self.w * (0.13 - 0.11 * y / self.h)
    }
}

/// Signed distance inside a rectangle with cut corners (positive inside).
fn chamfer_in(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, cut: f32) -> f32 {
    let (l, r, t, b) = (x - x0, x1 - x, y - y0, y1 - y);
    let edge = l.min(r).min(t).min(b);
    let c = |a: f32, b: f32| (a + b - cut) * std::f32::consts::FRAC_1_SQRT_2;
    edge.min(c(l, t)).min(c(r, t)).min(c(l, b)).min(c(r, b))
}

#[allow(clippy::too_many_arguments)]
fn light_rect(px: &mut [u32], w: usize, x0: i32, y0: i32, x1: i32, y1: i32, c: u32, k: u32) {
    let h = (px.len() / w) as i32;
    for y in y0.max(0)..y1.min(h) {
        for x in x0.max(0)..x1.min(w as i32) {
            let i = y as usize * w + x as usize;
            px[i] = add(px[i], c, k);
        }
    }
}

fn dash_colour(d: f32, dx: f32, s: f32) -> [f32; 3] {
    let (rim, surf) = (5.0 * s, 46.0 * s);
    let side = 1.0 - 0.55 * dx.abs().powi(3);
    let c = if d < 1.5 {
        hex(0x6E5CAE)
    } else if d < rim {
        mix3(hex(0x3A2E5C), hex(0x1A1430), (d - 1.5) / (rim - 1.5).max(0.1))
    } else if d < surf {
        // The sloping top of the dashboard catches the galaxy's light.
        let f = (d - rim) / (surf - rim);
        let mut c = mix3(hex(0x1C1630), hex(0x0E0B17), f);
        let sheen = (1.0 - f).powi(2) * (1.0 - dx.abs()).max(0.0) * 30.0;
        c[0] += sheen * 0.55;
        c[1] += sheen * 0.35;
        c[2] += sheen;
        c
    } else if d < surf + 2.0 * s {
        hex(0x040307)
    } else if d < surf + 3.0 * s {
        hex(0x2A2140)
    } else {
        mix3(hex(0x0D0B15), hex(0x060509), ((d - surf) / (140.0 * s)).min(1.0))
    };
    [c[0] * side, c[1] * side, c[2] * side]
}

/// Pillars and the canopy frame: a lit inner edge, a seam, dark metal.
fn frame_colour(e: f32, s: f32) -> [f32; 3] {
    if e < 2.0 * s {
        hex(0x4C3F7A)
    } else if e < 4.0 * s {
        hex(0x09070F)
    } else if (e - 26.0 * s).abs() < s {
        hex(0x050409)
    } else if (e - 27.8 * s).abs() < 0.6 * s {
        hex(0x241C36)
    } else {
        mix3(hex(0x1B1529), hex(0x07060B), ((e - 4.0 * s) / (70.0 * s)).min(1.0))
    }
}

pub struct Spaceship {
    w: usize,
    h: usize,
    s: f32,
    shape: Shape,
    dark: Vec<u32>,
    lit: Vec<u32>,
    cover: Vec<u8>,
    center: (f32, f32),
    particles: Vec<Particle>,
    screens: [Rect; 3],
    leds: Vec<Led>,
    sliders: Vec<Rect>,
    title: Sprite,
    title_glow: Sprite,
    sub: Sprite,
    log: Vec<Sprite>,
    online: Sprite,
    readouts: Vec<Sprite>,
    target: Vec<Sprite>,
    ticks: Vec<(i32, Sprite)>,
}

impl Spaceship {
    pub fn new(w: i32, h: i32) -> Spaceship {
        let (w, h) = (w.max(16) as usize, h.max(16) as usize);
        let (sw, sh) = (w as f32, h as f32);
        let s = sh / 800.0;
        let shape = Shape { w: sw, h: sh };
        let center = (sw / 2.0, sh * 0.30);
        let radius = sh * 0.36;
        let mut rng = Rng(0x0BA1_7E57);

        // Space: black with a faint violet haze and a vignette, and stars.
        let mut space = vec![0u32; w * h];
        let (bw, bh) = ((w / 8).max(1), (h / 8).max(1));
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
                let (dx, dy) = (x as f32 / sw - 0.5, y as f32 / sh - 0.35);
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
            if b > 0.7 && i > w && i + w + 1 < space.len() {
                for j in [i + 1, i - 1, i - w, i + w] {
                    space[j] = add(space[j], c, 60);
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
                let (u, v) = (dx * cr + dy * sr, (-dx * sr + dy * cr) / TILT);
                let d = (u * u + v * v).sqrt() / radius;
                let disc = (-d * 3.2).exp() * 0.75;
                let core = (-d * d * 260.0).exp() * 1.4 + (-d * 14.0).exp() * 0.9;
                let i = y * w + x;
                lit[i] = add(lit[i], 0x6A3FD8, (disc * 300.0) as u32);
                lit[i] = add(lit[i], 0xFFF4FF, (core * 255.0) as u32);
            }
        }

        // The bridge: canopy frame, pillars and dashboard, mitred where they
        // meet, with soft edges against the glass.
        let mut cover = vec![0u8; w * h];
        let mut bridge = vec![0u32; w * h];
        for y in 0..h {
            for x in 0..w {
                let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
                let pl = shape.pillar(yf);
                let into_dash = yf - shape.dash(xf);
                let into_top = shape.top(xf) - yf;
                let into_pillar = (pl - xf).max(xf - (sw - pl));
                let e = into_dash.max(into_top).max(into_pillar);
                let cov = (e + 0.5).clamp(0.0, 1.0);
                let i = y * w + x;
                if cov <= 0.0 {
                    // Glass: a faint diagonal reflection.
                    let u = (xf + yf * 0.6) / sw;
                    let band = (-((u - 0.30) / 0.05).powi(2)).exp() * 7.0 + (-((u - 0.38) / 0.012).powi(2)).exp() * 5.0;
                    if band > 0.5 {
                        space[i] = add(space[i], 0x8070C0, band as u32);
                        lit[i] = add(lit[i], 0x8070C0, band as u32);
                    }
                    continue;
                }
                let col = if into_dash >= into_top && into_dash >= into_pillar {
                    dash_colour(into_dash, shape.dx(xf), s)
                } else {
                    frame_colour(e, s)
                };
                cover[i] = (cov * 255.0) as u8;
                bridge[i] = pack(col);
            }
        }

        // Screens with cut-corner bezels.
        let screens = [
            Rect::frac(sw, sh, 0.135, 0.80, 0.19, 0.15),
            Rect::frac(sw, sh, 0.375, 0.785, 0.25, 0.175),
            Rect::frac(sw, sh, 0.675, 0.80, 0.19, 0.15),
        ];
        let bez = 10.0 * s;
        for r in &screens {
            let (x0, y0, x1, y1) = (r.x as f32, r.y as f32, (r.x + r.w) as f32, (r.y + r.h) as f32);
            let (bx0, by0, bx1, by1) = (x0 - bez, y0 - bez, x1 + bez, y1 + bez);
            for y in (by0.max(0.0) as usize)..(by1.ceil() as usize).min(h) {
                for x in (bx0.max(0.0) as usize)..(bx1.ceil() as usize).min(w) {
                    let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
                    let db = chamfer_in(xf, yf, bx0, by0, bx1, by1, 14.0 * s);
                    if db <= -0.5 {
                        continue;
                    }
                    let ds = chamfer_in(xf, yf, x0, y0, x1, y1, 7.0 * s);
                    let c = if ds > 1.0 {
                        0x07050D
                    } else if ds > 0.0 {
                        0x1A1428
                    } else if db < 1.5 * s {
                        0x2F2547
                    } else {
                        0x050409
                    };
                    let i = y * w + x;
                    bridge[i] = over(bridge[i], c, ((db + 0.5).clamp(0.0, 1.0) * 256.0) as u32);
                }
            }
        }

        // LED rows along the dashboard's top, in groups of six.
        let mut leds = Vec::new();
        let step = (10.0 * s).max(3.0);
        let (lw, lh) = ((6.0 * s).max(1.0) as i32, (3.0 * s).max(1.0) as i32);
        let mut i = 0;
        let mut x = sw * 0.07;
        while x < sw * 0.93 {
            if i % 8 < 6 {
                let y = (shape.dash(x) + 24.0 * s) as i32;
                let color = if i % 7 == 3 { WHITE } else if i % 11 == 5 { PINK } else { LILAC };
                let on = 0.75 + shape.dx(x).abs() * 0.55;
                leds.push(Led { x: x as i32, y, color, on });
                for yy in (y - 1).max(0)..(y + lh + 1).min(h as i32) {
                    for xx in (x as i32 - 1).max(0)..(x as i32 + lw + 1).min(w as i32) {
                        bridge[yy as usize * w + xx as usize] = 0x08060E;
                    }
                }
            }
            i += 1;
            x += step;
        }

        // Three slider slots at each end of the dashboard.
        let mut sliders = Vec::new();
        for k in 0..3 {
            let x = sw * 0.04 + k as f32 * 18.0 * s;
            for sx in [x, sw - x - 4.0 * s] {
                sliders.push(Rect { x: sx as i32, y: (sh * 0.85) as i32, w: (4.0 * s).max(1.0) as i32, h: (sh * 0.11) as i32 });
            }
        }
        for r in &sliders {
            for yy in (r.y - 1).max(0)..(r.y + r.h + 1).min(h as i32) {
                for xx in (r.x - 1).max(0)..(r.x + r.w + 1).min(w as i32) {
                    let edge = yy < r.y || yy >= r.y + r.h || xx < r.x || xx >= r.x + r.w;
                    bridge[yy as usize * w + xx as usize] = if edge { 0x241C36 } else { 0x030206 };
                }
            }
        }

        let mut dark = space;
        for i in 0..w * h {
            if cover[i] > 0 {
                let a = cover[i] as u32 + 1;
                dark[i] = over(dark[i], bridge[i], a);
                lit[i] = over(lit[i], bridge[i], a);
            }
        }

        // Particles in two main arms, two faint ones, a bulge and a halo.
        let mut particles = Vec::new();
        for i in 0..11000 {
            let kind = rng.f();
            let (rn, theta) = if kind < 0.18 {
                ((rng.f() * rng.f()) * 0.22, rng.f() * TAU)
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
                PINK
            } else if rng.below(3) == 0 {
                0xFFFFFF
            } else {
                pack(mix3(hex(0xB9A2FF), hex(0x6A5BFF), rn))
            };
            let k = ((60.0 + 200.0 * bright) * (0.6 + 0.4 * rng.f())) as u32;
            particles.push(Particle { r: rn * radius, theta, omega: 0.16 / (0.3 + rn * 2.0), color, k });
        }

        // Text: the title on the glass, the boot log on the centre screen,
        // readouts and a target label on the HUD.
        let sans = gfx::font_sans();
        let mono = gfx::font_mono();
        let tm = gfx::text_mask(&sans, 88.0 * s, 24.0 * s, crate::TITLE);
        let tx = (w as i32 - tm.w as i32) / 2;
        let ty = (sh * 0.545) as i32;
        let title = Sprite::from(tx, ty, &tm, |_, _| 0xFFFFFF);
        let tg = tm.padded(16).blurred(6, 3);
        let title_glow = Sprite::from(tx - 16, ty - 16, &tg, |_, _| VIOLET);
        let sm = gfx::text_mask(&mono, 18.0 * s, 8.0 * s, &format!("OS {}", crate::VERSION));
        let sub = Sprite::from((w as i32 - sm.w as i32) / 2, ty + tm.h as i32 + (12.0 * s) as i32, &sm, |_, _| LILAC);

        let c = screens[1];
        let log_lines = [format!("MPFREE OS {}", crate::VERSION), "NAV ........ OK".into(), "AUDIO ...... OK".into(), "PADS ....... OK".into()];
        let log = log_lines.iter().enumerate().map(|(i, l)| {
            let m = gfx::text_mask(&mono, 13.0 * s, 1.0 * s, l);
            let x = c.x + (12.0 * s) as i32;
            let y = c.y + (10.0 * s) as i32 + (i as f32 * 17.0 * s) as i32;
            Sprite::from(x, y, &m, |_, _| if i == 0 { WHITE } else { SOFT })
        }).collect();
        let om = gfx::text_mask(&mono, 15.0 * s, 3.0 * s, "SYSTEMS ONLINE");
        let online = Sprite::from(c.x + (c.w - om.w as i32) / 2, c.y + c.h - (34.0 * s) as i32, &om, |_, _| WHITE);

        let lines = ["NAV      LOCKED", "SECTOR   MPC-1", "DRIVE    IDLE", "COMMS    QUIET"];
        let readouts = lines.iter().enumerate().map(|(i, l)| {
            let m = gfx::text_mask(&mono, 14.0 * s, 1.5 * s, l);
            Sprite::from((sw * 0.15) as i32, (sh * 0.20 + i as f32 * 24.0 * s) as i32, &m, |_, _| SOFT)
        }).collect();
        let lines = ["TARGET  GALAXY 01", "RANGE   2.4 KLY", "LOCK    OK"];
        let target = lines.iter().enumerate().map(|(i, l)| {
            let m = gfx::text_mask(&mono, 14.0 * s, 1.5 * s, l);
            Sprite::from((sw * 0.72) as i32, (sh * 0.15 + i as f32 * 24.0 * s) as i32, &m, |_, _| if i == 2 { PINK } else { SOFT })
        }).collect();
        let ticks = (0..36).map(|i| {
            let m = gfx::text_mask(&mono, 12.0 * s, 1.0, &format!("{:03}", i * 10));
            (i * 10, Sprite::from(0, 0, &m, |_, _| 0x9D8CD6))
        }).collect();

        Spaceship {
            w, h, s, shape, dark, lit, cover, center, particles, screens, leds, sliders,
            title, title_glow, sub, log, online, readouts, target, ticks,
        }
    }

    /// The dashboard's rim lighting up from the middle outwards.
    fn rim(&self, px: &mut [u32], t: f32) {
        let sweep = smoothstep(0.5, 1.1, t) * 1.02;
        if sweep <= 0.0 {
            return;
        }
        let w = self.w;
        let seg = 8.0;
        let mut x = 0.0;
        while x < self.w as f32 {
            let x2 = x + seg;
            let d = self.shape.dx(x + seg / 2.0).abs();
            if d < sweep {
                let front = (1.0 - (sweep - d) / 0.08).clamp(0.0, 1.0) * if sweep < 1.0 { 1.0 } else { 0.0 };
                let k = (190.0 + 66.0 * front) as u32;
                let (y1, y2) = (self.shape.dash(x), self.shape.dash(x2));
                light_line(px, w, (x, y1 + 1.0), (x2, y2 + 1.0), LILAC, k, k);
                light_line(px, w, (x, y1 + 2.0), (x2, y2 + 2.0), VIOLET, k / 2, k / 2);
                light_line(px, w, (x, y1 - 1.0), (x2, y2 - 1.0), VIOLET, k / 4 + (front * 120.0) as u32, k / 4);
            }
            x = x2;
        }
    }

    fn panel(&self, px: &mut [u32], t: f32) {
        let (w, s) = (self.w, self.s);
        let (lw, lh) = ((6.0 * s).max(1.0) as i32, (3.0 * s).max(1.0) as i32);
        // LEDs, with a few blinking.
        for (i, l) in self.leds.iter().enumerate() {
            if t < l.on {
                continue;
            }
            let mut k = (smoothstep(l.on, l.on + 0.08, t) * 230.0) as u32;
            if t > 2.0 && hash(i as u32, (t * 5.0) as u32).is_multiple_of(9) {
                k /= 5;
            }
            light_rect(px, w, l.x, l.y, l.x + lw, l.y + lh, l.color, k);
            light_rect(px, w, l.x - 1, l.y - 1, l.x + lw + 1, l.y + lh + 1, l.color, k / 6);
        }
        // Sliders: settle to their levels once the panel is on.
        let on = smoothstep(0.9, 1.2, t);
        if on > 0.0 {
            for (i, r) in self.sliders.iter().enumerate() {
                let target = [0.72, 0.45, 0.6, 0.3, 0.85, 0.55][i % 6];
                let wobble = (1.0 - smoothstep(0.9, 2.4, t)) * 0.3 * (t * 5.0 + i as f32).sin();
                let lv = (target + wobble).clamp(0.05, 0.95);
                let ky = r.y + r.h - (lv * r.h as f32) as i32;
                light_rect(px, w, r.x, ky, r.x + r.w, r.y + r.h, VIOLET, (110.0 * on) as u32);
                let kw = (6.0 * s) as i32;
                light_rect(px, w, r.x - kw, ky - (2.0 * s) as i32, r.x + r.w + kw, ky + (3.0 * s) as i32, WHITE, (210.0 * on) as u32);
            }
        }
        for (j, r) in self.screens.iter().enumerate() {
            let start = [1.0, 0.85, 1.15][j];
            let p = smoothstep(start, start + 0.18, t);
            if p <= 0.0 {
                continue;
            }
            self.screen_on(px, *r, p);
            let ck = smoothstep(start + 0.15, start + 0.35, t);
            if ck <= 0.0 {
                continue;
            }
            match j {
                0 => self.radar(px, *r, t, ck),
                1 => self.boot_log(px, *r, t, start),
                _ => self.spectrum(px, *r, t, ck),
            }
        }
    }

    /// A screen opening like an old tube: a bright line that grows to fill it.
    fn screen_on(&self, px: &mut [u32], r: Rect, p: f32) {
        let (w, s) = (self.w, self.s);
        let cy = r.y as f32 + r.h as f32 / 2.0;
        let half = (r.h as f32 / 2.0 * p).max(1.0);
        let cut = (7.0 * s) as i32;
        let h = (px.len() / w) as i32;
        for y in ((cy - half) as i32).max(r.y).max(0)..((cy + half) as i32).min(r.y + r.h).min(h) {
            let edge = (y - r.y).min(r.y + r.h - 1 - y);
            let inset = (cut - edge).max(0) + 1;
            let c = if (y - r.y) % 3 == 0 { 0x0C0718 } else { 0x140B2A };
            for x in (r.x + inset).max(0)..(r.x + r.w - inset).min(w as i32) {
                px[y as usize * w + x as usize] = c;
            }
        }
        if p < 1.0 {
            let k = ((1.0 - p) * 256.0) as u32;
            light_line(px, w, (r.x as f32 + 4.0, cy), ((r.x + r.w) as f32 - 4.0, cy), WHITE, k, k);
        }
    }

    fn radar(&self, px: &mut [u32], r: Rect, t: f32, ck: f32) {
        let (w, s) = (self.w, self.s);
        let rad = (r.h as f32 / 2.0 - 8.0 * s).max(3.0);
        let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
        let kk = |k: f32| (k * ck) as u32;
        // A faint grid behind.
        let g = (16.0 * s).max(4.0) as i32;
        for x in (r.x + g..r.x + r.w - 2).step_by(g as usize) {
            light_line(px, w, (x as f32, r.y as f32 + 3.0), (x as f32, (r.y + r.h) as f32 - 3.0), VIOLET, kk(22.0), kk(22.0));
        }
        for y in (r.y + g..r.y + r.h - 2).step_by(g as usize) {
            light_line(px, w, (r.x as f32 + 3.0, y as f32), ((r.x + r.w) as f32 - 3.0, y as f32), VIOLET, kk(22.0), kk(22.0));
        }
        for ring in [1.0, 0.66, 0.33] {
            let rr = rad * ring;
            let n = (TAU * rr) as usize + 1;
            for i in 0..n {
                let a = i as f32 / n as f32 * TAU;
                let (x, y) = ((cx + a.cos() * rr) as usize, (cy + a.sin() * rr) as usize);
                let j = y * w + x;
                if j < px.len() {
                    px[j] = add(px[j], LILAC, kk(110.0));
                }
            }
        }
        light_line(px, w, (cx - rad, cy), (cx + rad, cy), LILAC, kk(50.0), kk(50.0));
        light_line(px, w, (cx, cy - rad), (cx, cy + rad), LILAC, kk(50.0), kk(50.0));
        let a = t * 2.4;
        for m in 0..28 {
            let aa = a - m as f32 * 0.03;
            let k = kk(220.0 - m as f32 * 7.5);
            light_line(px, w, (cx, cy), (cx + aa.cos() * rad, cy + aa.sin() * rad), if m == 0 { WHITE } else { LILAC }, k / 3, k);
        }
        for (ang, rn) in [(0.6, 0.7), (2.1, 0.4), (3.3, 0.85), (4.6, 0.55), (5.5, 0.25)] {
            let since = (a - ang).rem_euclid(TAU);
            let k = kk((-since * 1.1).exp() * 256.0);
            let (x, y) = ((cx + ang.cos() * rad * rn) as i32, (cy + ang.sin() * rad * rn) as i32);
            light_rect(px, w, x - 1, y - 1, x + 2, y + 2, PINK, k);
            light_rect(px, w, x, y, x + 1, y + 1, WHITE, k);
        }
    }

    fn boot_log(&self, px: &mut [u32], r: Rect, t: f32, start: f32) {
        let (w, s) = (self.w, self.s);
        for (i, l) in self.log.iter().enumerate() {
            let at = start + 0.3 + i as f32 * 0.22;
            if t > at {
                l.draw_upto(px, w, 256, true, ((t - at) * 900.0 * s) as i32);
            }
        }
        let done = 2.7;
        let (x0, x1) = (r.x as f32 + 12.0 * s, (r.x + r.w) as f32 - 12.0 * s);
        let y = (r.y + r.h) as f32 - 30.0 * s;
        let bh = (8.0 * s).max(2.0);
        if t < done {
            let p = smoothstep(1.6, 2.6, t);
            let k = (smoothstep(1.3, 1.5, t) * 256.0) as u32;
            if k == 0 {
                return;
            }
            for (a, b) in [((x0, y), (x1, y)), ((x0, y + bh), (x1, y + bh)), ((x0, y), (x0, y + bh)), ((x1, y), (x1, y + bh))] {
                light_line(px, w, a, b, LILAC, (k * 150) >> 8, (k * 150) >> 8);
            }
            // Filled in blocks.
            let block = (8.0 * s).max(3.0);
            let mut x = x0 + 2.0;
            while x + block - 2.0 < x0 + 2.0 + (x1 - x0 - 4.0) * p {
                light_rect(px, w, x as i32, (y + 2.0) as i32, (x + block - 2.0) as i32, (y + bh - 1.0) as i32, LILAC, (k * 230) >> 8);
                x += block;
            }
        } else if t < 3.0 || (t * 1.6).fract() < 0.8 {
            self.online.draw(px, w, 256, true);
        }
    }

    fn spectrum(&self, px: &mut [u32], r: Rect, t: f32, ck: f32) {
        let (w, s) = (self.w, self.s);
        let n = 20;
        let pad = 8.0 * s;
        let bw = (r.w as f32 - 2.0 * pad) / n as f32;
        let base = (r.y + r.h) as f32 - pad;
        let top = r.y as f32 + r.h as f32 * 0.4;
        let row = (4.0 * s).max(2.0) as i32;
        for i in 0..n {
            let f = i as f32;
            let lv = 0.15 + 0.8 * ((t * 2.7 + f * 1.3).sin() * (t * 1.1 + f * 0.37).sin()).abs();
            let hgt = (base - top) * lv * ck;
            let x0 = (r.x as f32 + pad + f * bw) as i32;
            let x1 = (r.x as f32 + pad + (f + 1.0) * bw - 2.0 * s) as i32;
            let mut y = base as i32;
            while y as f32 > base - hgt {
                let up = (base - y as f32) / (base - top);
                let c = if up > 0.8 { WHITE } else if up > 0.55 { PINK } else { LILAC };
                light_rect(px, w, x0, y - row + 1, x1, y, c, (200.0 * ck) as u32);
                y -= row;
            }
        }
        // A waveform along the top.
        let wy = r.y as f32 + r.h as f32 * 0.2;
        let amp = r.h as f32 * 0.13;
        let mut prev: Option<(f32, f32)> = None;
        let mut x = r.x as f32 + pad;
        while x < (r.x + r.w) as f32 - pad {
            let y = wy + (x * 0.09 / s + t * 7.0).sin() * (x * 0.021 / s - t * 1.7).sin() * amp;
            if let Some(p) = prev {
                light_line(px, w, p, (x, y), PINK, (220.0 * ck) as u32, (220.0 * ck) as u32);
            }
            prev = Some((x, y));
            x += 2.0;
        }
    }

    fn hud(&self, px: &mut [u32], t: f32) {
        let (w, s) = (self.w, self.s);
        let sw = w as f32;
        let k = smoothstep(1.7, 2.1, t);
        if k <= 0.0 {
            return;
        }
        let kk = (k * 256.0) as u32;
        let hk = |a: u32| (a * kk) >> 8;
        // Corner brackets that slide in to frame the view.
        let out = (1.0 - k) * 30.0 * s;
        let (bx0, bx1) = (sw * 0.2 - out, sw * 0.8 + out);
        let (by0, by1) = (self.shape.top(sw * 0.2) + 24.0 * s - out, self.shape.dash(sw * 0.8) - 26.0 * s + out);
        let arm = 26.0 * s;
        for (x, y, dx, dy) in [(bx0, by0, 1.0, 1.0), (bx1, by0, -1.0, 1.0), (bx0, by1, 1.0, -1.0), (bx1, by1, -1.0, -1.0)] {
            light_line(px, w, (x, y), (x + dx * arm, y), LILAC, hk(170), hk(60));
            light_line(px, w, (x, y), (x, y + dy * arm), LILAC, hk(170), hk(60));
        }
        // Heading tape along the top, drifting slowly.
        let (y0, x0, x1) = (self.shape.top(sw / 2.0) + 30.0 * s, sw * 0.36, sw * 0.64);
        light_line(px, w, (x0, y0), (x1, y0), LILAC, hk(90), hk(90));
        let heading = 172.0 + (t - 1.7).max(0.0) * 1.5;
        let spacing = 8.0 * s;
        let first = (heading - (x1 - x0) / spacing / 2.0).floor() as i32;
        for d in first..first + ((x1 - x0) / spacing) as i32 + 2 {
            let x = sw / 2.0 + (d as f32 - heading) * spacing;
            if x < x0 || x > x1 {
                continue;
            }
            let fade = 1.0 - ((x - sw / 2.0).abs() / ((x1 - x0) / 2.0)).powi(2);
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
        let cx = sw / 2.0;
        light_line(px, w, (cx - 6.0 * s, y0 - 10.0 * s), (cx, y0 - 2.0 * s), WHITE, hk(220), hk(220));
        light_line(px, w, (cx + 6.0 * s, y0 - 10.0 * s), (cx, y0 - 2.0 * s), WHITE, hk(220), hk(220));

        // A reticle that closes on the galaxy's core, turning.
        let (gx, gy) = self.center;
        let close = smoothstep(1.9, 2.6, t);
        let rr = 54.0 * s * (1.0 + 1.4 * (1.0 - close));
        let spin = t * 0.25;
        for q in 0..4 {
            let a = spin + q as f32 * PI / 2.0 + PI / 4.0;
            let (ca, sa) = (a.cos(), a.sin());
            let p = (gx + ca * rr, gy + sa * rr);
            let arm = 14.0 * s;
            let (t1, t2) = ((-sa, ca), (-ca, -sa));
            let k2 = hk(200);
            light_line(px, w, p, (p.0 + t1.0 * arm, p.1 + t1.1 * arm), WHITE, k2, k2);
            light_line(px, w, p, (p.0 - t1.0 * arm, p.1 - t1.1 * arm), WHITE, k2, k2);
            light_line(px, w, p, (p.0 + t2.0 * arm * 0.7, p.1 + t2.1 * arm * 0.7), WHITE, k2, k2);
        }
        // Once locked: a leader line out to the target label.
        let lk = smoothstep(2.5, 2.7, t);
        if lk > 0.0 {
            let from = (gx + rr * 0.8, gy - rr * 0.8);
            let elbow = (sw * 0.71, self.target[0].y as f32 + 7.0 * s);
            let end = (from.0 + (elbow.0 - from.0) * lk, from.1 + (elbow.1 - from.1) * lk);
            light_line(px, w, from, end, LILAC, (160.0 * lk) as u32, (160.0 * lk) as u32);
            for (i, l) in self.target.iter().enumerate() {
                let at = 2.6 + i as f32 * 0.15;
                if t > at {
                    l.draw_upto(px, w, 256, true, ((t - at) * 900.0 * s) as i32);
                }
            }
        }
        for (i, r) in self.readouts.iter().enumerate() {
            let at = 2.0 + i as f32 * 0.15;
            if t > at {
                r.draw_upto(px, w, 256, true, ((t - at) * 900.0 * s) as i32);
            }
        }
    }
}

impl Scene for Spaceship {
    fn length(&self) -> f32 {
        3.8
    }

    fn draw(&mut self, t: f32, out: &mut SoftCanvas) {
        let w = self.w;
        let h = self.h;
        let px = &mut out.px;
        // The view fades up out of the dark and the galaxy's glow with it.
        let glow = smoothstep(0.0, 1.4, t);
        let bright = smoothstep(0.0, 0.6, t);
        if glow >= 1.0 {
            px.copy_from_slice(&self.lit);
        } else {
            let (kg, kb) = ((glow * 256.0) as u32, (bright * 256.0) as u32);
            for ((p, &a), &b) in px.iter_mut().zip(&self.dark).zip(&self.lit) {
                *p = scale(over(a, b, kg), kb);
            }
        }

        // The galaxy, turning, hidden where the bridge is in front.
        let (cx, cy) = self.center;
        let (cr, sr) = (ROLL.cos(), ROLL.sin());
        let kf = (smoothstep(0.0, 1.2, t) * 256.0) as u32;
        for p in &self.particles {
            let a = p.theta + p.omega * t;
            let (u, v) = (a.cos() * p.r, a.sin() * p.r * TILT);
            let x = cx + u * cr - v * sr;
            let y = cy + u * sr + v * cr;
            if x < 1.0 || y < 1.0 || x >= (w - 2) as f32 || y >= (h - 2) as f32 {
                continue;
            }
            let i = y as usize * w + x as usize;
            if self.cover[i] > 0 {
                continue;
            }
            let k = (p.k * kf) >> 8;
            px[i] = add(px[i], p.color, k);
            if p.k > 150 {
                let k2 = k / 3;
                px[i + 1] = add(px[i + 1], p.color, k2);
                px[i + w] = add(px[i + w], p.color, k2);
            }
        }

        self.rim(px, t);
        self.panel(px, t);
        self.hud(px, t);

        // Title: wiped onto the glass by a scan line, then the subtitle.
        if t > 2.6 {
            let wipe = smoothstep(2.6, 3.2, t);
            let upto = (wipe * (self.title_glow.w as f32 + 40.0)) as i32;
            self.title_glow.draw_upto(px, w, 230, true, upto);
            self.title.draw_upto(px, w, 256, false, upto - 16);
            if wipe < 1.0 {
                let x = (self.title_glow.x + upto) as f32;
                let (y0, y1) = (self.title.y as f32 - 20.0 * self.s, self.title.y as f32 + 100.0 * self.s);
                for dx in -2i32..=2 {
                    let k = [60, 140, 256, 140, 60][(dx + 2) as usize];
                    light_line(px, w, (x + dx as f32, y0), (x + dx as f32, y1), WHITE, k, k);
                }
            }
        }
        if t > 3.1 {
            let upto = ((t - 3.1) * 1400.0 * self.s) as i32;
            self.sub.draw_upto(px, w, 256, true, upto);
        }
    }
}
