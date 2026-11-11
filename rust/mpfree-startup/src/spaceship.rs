//! The spaceship theme's startup screen: a purple spiral galaxy in open
//! space, with a sci-fi computer overlay analysing it. There is no ship in
//! the picture, only the data laid over the view.
//!
//! - 0.0 s: the galaxy fades up out of the dark, turning slowly (inner stars
//!   faster than outer ones).
//! - 0.6 s: a heading tape and corner brackets come up; four translucent
//!   panels open: a radar scanner, a spectrum with a waveform, navigation
//!   readouts and a target panel.
//! - 1.0 s: a scan line sweeps across the galaxy, tagging stars with small
//!   callouts as it passes; a status bar fills along the bottom.
//! - 1.8 s: a reticle closes on the galaxy's core and a leader line runs to
//!   the target panel, which types out what it found.
//! - 2.4 s: "MPFree OS" is cut on in the blade letters, white to violet, then
//!   "v0.1" flickers on and the status bar reads ALL SYSTEMS ONLINE.
//!
//! Made once: the star field and the galaxy's glow (with and without it, to
//! fade between), ~11,000 galaxy particles and all the text. A frame copies
//! the background, adds the particles, then draws the panels (small
//! translucent rectangles) and the overlay's thin lines.

use crate::blade;
use crate::gfx::{
    self, add, hex, light_line, mix3, over, pack, scale, smoothstep, Mask, Rng, Sprite,
};
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
        Rect {
            x: (sw * x) as i32,
            y: (sh * y) as i32,
            w: (sw * w).max(8.0) as i32,
            h: (sh * h).max(8.0) as i32,
        }
    }
}

/// A translucent panel of the overlay: when it opens, its header and what
/// it shows.
struct Panel {
    r: Rect,
    open: f32,
    header: Sprite,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Radar,
    Spectrum,
    Nav,
    Target,
}

/// A star the scan line tags: which particle, and its label.
struct Callout {
    particle: usize,
    label: Sprite,
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

/// A neon tube striking: on, off, on, flickering, then on for good.
fn neon(t: f32, start: f32) -> u32 {
    let d = t - start;
    if d < 0.0 {
        return 0;
    }
    const STEPS: [(f32, u32); 8] = [
        (0.05, 200),
        (0.1, 0),
        (0.16, 256),
        (0.22, 40),
        (0.3, 256),
        (0.34, 90),
        (0.42, 256),
        (9e9, 256),
    ];
    STEPS.iter().find(|s| d < s.0).map(|s| s.1).unwrap_or(256)
}

pub struct Spaceship {
    w: usize,
    h: usize,
    s: f32,
    dark: Vec<u32>,
    lit: Vec<u32>,
    center: (f32, f32),
    particles: Vec<Particle>,
    panels: Vec<Panel>,
    callouts: Vec<Callout>,
    title: Sprite,
    title_h: i32,
    title_edge: Sprite,
    title_glow: Sprite,
    sub: Sprite,
    sub_glow: Sprite,
    readouts: Vec<Sprite>,
    target: Vec<Sprite>,
    online: Sprite,
    ticks: Vec<(i32, Sprite)>,
}

impl Spaceship {
    pub fn new(w: i32, h: i32) -> Spaceship {
        let (w, h) = (w.max(16) as usize, h.max(16) as usize);
        let (sw, sh) = (w as f32, h as f32);
        let s = sh / 800.0;
        let center = (sw / 2.0, sh * 0.40);
        let radius = sh * 0.40;
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
                let (dx, dy) = (x as f32 / sw - 0.5, y as f32 / sh - 0.42);
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
        let dark = space;

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
            particles.push(Particle {
                r: rn * radius,
                theta,
                omega: 0.16 / (0.3 + rn * 2.0),
                color,
                k,
            });
        }

        let mono = gfx::font_mono();
        let small = |text: &str, c: u32, x: i32, y: i32| {
            let m = gfx::text_mask(&mono, 13.0 * s, 1.5 * s, text);
            Sprite::from(x, y, &m, |_, _| c)
        };

        // The overlay's panels.
        let specs = [
            (Kind::Radar, "SCAN", (0.03, 0.30, 0.17, 0.27), 0.8),
            (Kind::Spectrum, "SPECTRUM", (0.80, 0.30, 0.17, 0.27), 0.95),
            (Kind::Nav, "NAV", (0.03, 0.05, 0.19, 0.17), 1.1),
            (Kind::Target, "TARGET", (0.78, 0.05, 0.19, 0.17), 1.25),
        ];
        let pad = (10.0 * s) as i32;
        let panels: Vec<Panel> = specs
            .iter()
            .map(|&(kind, name, (x, y, pw, ph), open)| {
                let r = Rect::frac(sw, sh, x, y, pw, ph);
                Panel {
                    r,
                    open,
                    header: small(name, WHITE, r.x + pad, r.y + (7.0 * s) as i32),
                    kind,
                }
            })
            .collect();
        let body = |k: Kind| {
            let p = panels.iter().find(|p| p.kind == k).expect("panel");
            (p.r.x + pad, p.r.y + (30.0 * s) as i32)
        };
        let (nx, ny) = body(Kind::Nav);
        let readouts = [
            "NAV      LOCKED",
            "SECTOR   MPC-1",
            "DRIVE    IDLE",
            "COMMS    QUIET",
        ]
        .iter()
        .enumerate()
        .map(|(i, l)| small(l, SOFT, nx, ny + (i as f32 * 19.0 * s) as i32))
        .collect();
        let (tx0, ty0) = body(Kind::Target);
        let target = [
            "CLASS    SPIRAL SBc",
            "RANGE    2.4 KLY",
            "STARS    4.1E11",
            "LOCK     OK",
        ]
        .iter()
        .enumerate()
        .map(|(i, l)| {
            small(
                l,
                if i == 3 { PINK } else { SOFT },
                tx0,
                ty0 + (i as f32 * 19.0 * s) as i32,
            )
        })
        .collect();
        let om = gfx::text_mask(&mono, 14.0 * s, 4.0 * s, "ALL SYSTEMS ONLINE");
        let online = Sprite::from(
            (w as i32 - om.w as i32) / 2,
            (sh * 0.935) as i32,
            &om,
            |_, _| WHITE,
        );
        let ticks = (0..36)
            .map(|i| {
                let m = gfx::text_mask(&mono, 12.0 * s, 1.0, &format!("{:03}", i * 10));
                (i * 10, Sprite::from(0, 0, &m, |_, _| 0x9D8CD6))
            })
            .collect();

        // Stars the scan line tags: bright particles out in the arms, spread
        // round the disc.
        let names = ["HII-07", "CL 0x3F", "SNR 12", "V 2.41", "OB-221", "PSR 0.8"];
        let mut callouts = Vec::new();
        for (n, name) in names.iter().enumerate() {
            let want = n as f32 / names.len() as f32 * TAU + 0.4;
            let near = |p: &Particle| {
                let d = (p.theta - want).rem_euclid(TAU);
                d.min(TAU - d) < 0.35
            };
            let found = particles.iter().enumerate().position(|(_, p)| {
                let rn = p.r / radius;
                p.k > 95 && (0.32 + 0.08 * (n % 3) as f32..0.8).contains(&rn) && near(p)
            });
            if let Some(i) = found {
                callouts.push(Callout {
                    particle: i,
                    label: small(name, WHITE, 0, 0),
                });
            }
        }

        // The title: "MPFree OS" in the synthwave screen's blade letters,
        // white to violet.
        let text = format!("{} OS", crate::TITLE);
        let max_w = sw * 0.62;
        let style = blade::Style {
            seed: 3,
            ..Default::default()
        };
        let mut cap = 92.0 * s;
        let mut tm = blade::render(&text, cap, style);
        if tm.w as f32 > max_w {
            cap *= max_w / tm.w as f32;
            tm = blade::render(&text, cap, style);
        }
        let th = tm.h as f32;
        let tx = (w as i32 - tm.w as i32) / 2;
        let ty = (sh * 0.685) as i32;
        let title = Sprite::from(tx, ty, &tm, |_, y| {
            let v = y as f32 / th;
            pack(if v < 0.45 {
                mix3(hex(0xFFFFFF), hex(0xE2D6FF), v / 0.45)
            } else {
                mix3(hex(0xE2D6FF), hex(0x9B6BFF), (v - 0.45) / 0.55)
            })
        });
        let pad = 24;
        let edge = tm.padded(pad).dilated(3);
        let title_edge = Sprite::from(tx - pad as i32, ty - pad as i32, &edge, |_, _| 0x0E0620);
        let glow = edge.blurred(9, 3);
        let title_glow = Sprite::from(tx - pad as i32, ty - pad as i32, &glow, |_, _| VIOLET);
        let sm = blade::render(crate::VERSION, 30.0 * s, blade::Style { seed: 5, ..style });
        let (sx, sy) = (
            (w as i32 - sm.w as i32) / 2,
            ty + tm.h as i32 + (8.0 * s) as i32,
        );
        let sub = Sprite::from(sx, sy, &sm, |_, _| WHITE);
        let sub_glow = Sprite::from(sx - 12, sy - 12, &sm.padded(12).blurred(5, 3), |_, _| LILAC);

        Spaceship {
            w,
            h,
            s,
            dark,
            lit,
            center,
            particles,
            panels,
            callouts,
            title,
            title_h: tm.h as i32,
            title_edge,
            title_glow,
            sub,
            sub_glow,
            readouts,
            target,
            online,
            ticks,
        }
    }

    /// Where particle `p` is on screen at time `t`.
    fn place(&self, p: &Particle, t: f32) -> (f32, f32) {
        let (cx, cy) = self.center;
        let (cr, sr) = (ROLL.cos(), ROLL.sin());
        let a = p.theta + p.omega * t;
        let (u, v) = (a.cos() * p.r, a.sin() * p.r * TILT);
        (cx + u * cr - v * sr, cy + u * sr + v * cr)
    }

    /// A panel: it unfolds sideways from its middle, a dark translucent body
    /// with a cut-corner outline, a header and a rule under it.
    fn panel(&self, px: &mut [u32], p: &Panel, t: f32) -> f32 {
        let (w, s) = (self.w, self.s);
        let o = smoothstep(p.open, p.open + 0.22, t);
        if o <= 0.0 {
            return 0.0;
        }
        let r = p.r;
        let half = r.w as f32 / 2.0 * o;
        let mid = r.x as f32 + r.w as f32 / 2.0;
        let (x0, x1, y0, y1) = (mid - half, mid + half, r.y as f32, (r.y + r.h) as f32);
        let cut = 10.0 * s;
        let h = px.len() / w;
        for y in (y0.max(0.0) as usize)..(y1 as usize).min(h) {
            for x in (x0.max(0.0) as usize)..(x1.ceil() as usize).min(w) {
                let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
                let d = chamfer_in(xf, yf, x0, y0, x1, y1, cut);
                if d <= -0.5 {
                    continue;
                }
                let i = y * w + x;
                let a = if d < 1.0 { 0 } else { 150 };
                px[i] = over(px[i], 0x0A0616, a);
                if d < 1.2 {
                    px[i] = add(
                        px[i],
                        LILAC,
                        (((1.2 - d.abs()).clamp(0.0, 1.0)) * 200.0) as u32,
                    );
                }
            }
        }
        // Scanlines over the body, faint.
        let mut y = r.y + 2;
        while y < r.y + r.h - 2 {
            light_line(
                px,
                w,
                (x0 + 3.0, y as f32),
                (x1 - 3.0, y as f32),
                VIOLET,
                10,
                10,
            );
            y += (3.0 * s).max(2.0) as i32;
        }
        let ck = smoothstep(p.open + 0.15, p.open + 0.35, t);
        if ck > 0.0 {
            p.header.draw(px, w, (ck * 256.0) as u32, true);
            let ry = r.y as f32 + 24.0 * s;
            light_line(
                px,
                w,
                (x0 + 8.0 * s, ry),
                (x1 - 8.0 * s, ry),
                LILAC,
                (90.0 * ck) as u32,
                (20.0 * ck) as u32,
            );
        }
        ck
    }

    fn radar(&self, px: &mut [u32], r: Rect, t: f32, ck: f32) {
        let (w, s) = (self.w, self.s);
        let top = r.y as f32 + 28.0 * s;
        let rad = ((r.h as f32 - 28.0 * s) / 2.0 - 8.0 * s)
            .min(r.w as f32 / 2.0 - 10.0 * s)
            .max(3.0);
        let (cx, cy) = (
            r.x as f32 + r.w as f32 / 2.0,
            top + (r.y as f32 + r.h as f32 - top) / 2.0,
        );
        let kk = |k: f32| (k * ck) as u32;
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
        light_line(
            px,
            w,
            (cx - rad, cy),
            (cx + rad, cy),
            LILAC,
            kk(50.0),
            kk(50.0),
        );
        light_line(
            px,
            w,
            (cx, cy - rad),
            (cx, cy + rad),
            LILAC,
            kk(50.0),
            kk(50.0),
        );
        let a = t * 2.4;
        for m in 0..28 {
            let aa = a - m as f32 * 0.03;
            let k = kk(220.0 - m as f32 * 7.5);
            light_line(
                px,
                w,
                (cx, cy),
                (cx + aa.cos() * rad, cy + aa.sin() * rad),
                if m == 0 { WHITE } else { LILAC },
                k / 3,
                k,
            );
        }
        for (ang, rn) in [
            (0.6, 0.7),
            (2.1, 0.4),
            (3.3, 0.85),
            (4.6, 0.55),
            (5.5, 0.25),
        ] {
            let since = (a - ang).rem_euclid(TAU);
            let k = kk((-since * 1.1).exp() * 256.0);
            let (x, y) = (
                (cx + ang.cos() * rad * rn) as i32,
                (cy + ang.sin() * rad * rn) as i32,
            );
            light_rect(px, w, x - 1, y - 1, x + 2, y + 2, PINK, k);
            light_rect(px, w, x, y, x + 1, y + 1, WHITE, k);
        }
    }

    fn spectrum(&self, px: &mut [u32], r: Rect, t: f32, ck: f32) {
        let (w, s) = (self.w, self.s);
        let n = 16;
        let pad = 10.0 * s;
        let bw = (r.w as f32 - 2.0 * pad) / n as f32;
        let base = (r.y + r.h) as f32 - pad;
        let top = r.y as f32 + r.h as f32 * 0.45;
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
                let c = if up > 0.8 {
                    WHITE
                } else if up > 0.55 {
                    PINK
                } else {
                    LILAC
                };
                light_rect(px, w, x0, y - row + 1, x1, y, c, (200.0 * ck) as u32);
                y -= row;
            }
        }
        // A waveform above the bars.
        let wy = r.y as f32 + r.h as f32 * 0.3;
        let amp = r.h as f32 * 0.09;
        let mut prev: Option<(f32, f32)> = None;
        let mut x = r.x as f32 + pad;
        while x < (r.x + r.w) as f32 - pad {
            let y = wy + (x * 0.09 / s + t * 7.0).sin() * (x * 0.021 / s - t * 1.7).sin() * amp;
            if let Some(p) = prev {
                light_line(
                    px,
                    w,
                    p,
                    (x, y),
                    PINK,
                    (220.0 * ck) as u32,
                    (220.0 * ck) as u32,
                );
            }
            prev = Some((x, y));
            x += 2.0;
        }
    }

    fn type_lines(&self, px: &mut [u32], lines: &[Sprite], t: f32, start: f32) {
        for (i, l) in lines.iter().enumerate() {
            let at = start + i as f32 * 0.15;
            if t > at {
                l.draw_upto(px, self.w, 256, true, ((t - at) * 900.0 * self.s) as i32);
            }
        }
    }

    /// The scan line sweeping the galaxy, and the stars it tags.
    fn scan(&self, px: &mut [u32], t: f32) {
        let (w, s) = (self.w, self.s);
        let (x0, x1) = (w as f32 * 0.24, w as f32 * 0.76);
        let (y0, y1) = (self.h as f32 * 0.12, self.h as f32 * 0.66);
        let p = smoothstep(1.0, 2.0, t);
        let at = x0 + (x1 - x0) * p;
        if p > 0.0 && p < 1.0 {
            for d in 0..(60.0 * s) as i32 {
                let k = (120.0 * (1.0 - d as f32 / (60.0 * s))).powf(1.0) as u32 / 3;
                let x = at - d as f32;
                light_line(px, w, (x, y0), (x, y1), VIOLET, k / 3, k);
            }
            light_line(px, w, (at, y0), (at, y1), WHITE, 120, 220);
        }
        for c in &self.callouts {
            let (x, y) = self.place(&self.particles[c.particle], t);
            // The time the line passed it.
            let hit = 1.0 + (((x - x0) / (x1 - x0)).clamp(0.0, 1.0)) * 1.0;
            if t < hit || x < x0 || x > x1 {
                continue;
            }
            let age = t - hit;
            let k = if age < 0.12 {
                256.0
            } else {
                120.0 + 136.0 * (-(age - 0.12) * 2.0).exp()
            };
            let kk = k as u32;
            let b = 6.0 * s * (1.0 + 1.5 * (1.0 - smoothstep(0.0, 0.15, age)));
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let (cx, cy) = (x + dx * b, y + dy * b);
                light_line(px, w, (cx, cy), (cx - dx * b * 0.6, cy), WHITE, kk, kk);
                light_line(px, w, (cx, cy), (cx, cy - dy * b * 0.6), WHITE, kk, kk);
            }
            let lx = x + 16.0 * s;
            let ly = y - 16.0 * s;
            light_line(px, w, (x + b, y - b), (lx, ly), LILAC, kk / 2, kk / 2);
            light_line(
                px,
                w,
                (lx, ly),
                (lx + c.label.w as f32 + 4.0, ly),
                LILAC,
                kk / 2,
                kk / 3,
            );
            let upto = (age * 900.0 * s) as i32;
            c.label.draw_upto_at(
                px,
                w,
                lx as i32 + 2,
                ly as i32 - c.label.h() as i32 - 2,
                kk,
                upto,
            );
        }
    }

    fn hud(&self, px: &mut [u32], t: f32) {
        let (w, s) = (self.w, self.s);
        let sw = w as f32;
        let k = smoothstep(0.6, 1.0, t);
        if k <= 0.0 {
            return;
        }
        let kk = (k * 256.0) as u32;
        let hk = |a: u32| (a * kk) >> 8;
        // Corner brackets that slide in to frame the galaxy.
        let out = (1.0 - k) * 30.0 * s;
        let (bx0, bx1) = (sw * 0.24 - out, sw * 0.76 + out);
        let (by0, by1) = (self.h as f32 * 0.12 - out, self.h as f32 * 0.66 + out);
        let arm = 26.0 * s;
        for (x, y, dx, dy) in [
            (bx0, by0, 1.0, 1.0),
            (bx1, by0, -1.0, 1.0),
            (bx0, by1, 1.0, -1.0),
            (bx1, by1, -1.0, -1.0),
        ] {
            light_line(px, w, (x, y), (x + dx * arm, y), LILAC, hk(170), hk(60));
            light_line(px, w, (x, y), (x, y + dy * arm), LILAC, hk(170), hk(60));
        }
        // Heading tape along the top, drifting slowly.
        let (y0, x0, x1) = (self.h as f32 * 0.05, sw * 0.36, sw * 0.64);
        light_line(px, w, (x0, y0), (x1, y0), LILAC, hk(90), hk(90));
        let heading = 172.0 + (t - 0.6).max(0.0) * 1.5;
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
                    spr.draw_at(
                        px,
                        w,
                        x as i32 - spr.w as i32 / 2,
                        (y0 + 16.0 * s) as i32,
                        a.min(256),
                    );
                }
            }
        }
        let cx = sw / 2.0;
        light_line(
            px,
            w,
            (cx - 6.0 * s, y0 - 10.0 * s),
            (cx, y0 - 2.0 * s),
            WHITE,
            hk(220),
            hk(220),
        );
        light_line(
            px,
            w,
            (cx + 6.0 * s, y0 - 10.0 * s),
            (cx, y0 - 2.0 * s),
            WHITE,
            hk(220),
            hk(220),
        );

        // A reticle that closes on the galaxy's core, turning.
        let rk = smoothstep(1.7, 1.9, t);
        if rk <= 0.0 {
            return;
        }
        let (gx, gy) = self.center;
        let close = smoothstep(1.8, 2.4, t);
        let rr = 54.0 * s * (1.0 + 1.4 * (1.0 - close));
        let spin = t * 0.25;
        let k2 = (200.0 * rk) as u32;
        for q in 0..4 {
            let a = spin + q as f32 * PI / 2.0 + PI / 4.0;
            let (ca, sa) = (a.cos(), a.sin());
            let p = (gx + ca * rr, gy + sa * rr);
            let arm = 14.0 * s;
            let (t1, t2) = ((-sa, ca), (-ca, -sa));
            light_line(
                px,
                w,
                p,
                (p.0 + t1.0 * arm, p.1 + t1.1 * arm),
                WHITE,
                k2,
                k2,
            );
            light_line(
                px,
                w,
                p,
                (p.0 - t1.0 * arm, p.1 - t1.1 * arm),
                WHITE,
                k2,
                k2,
            );
            light_line(
                px,
                w,
                p,
                (p.0 + t2.0 * arm * 0.7, p.1 + t2.1 * arm * 0.7),
                WHITE,
                k2,
                k2,
            );
        }
        // Once locked: a leader line out to the target panel.
        let lk = smoothstep(2.3, 2.5, t);
        if lk > 0.0 {
            if let Some(p) = self.panels.iter().find(|p| p.kind == Kind::Target) {
                let from = (gx + rr * 0.8, gy - rr * 0.8);
                let to = (p.r.x as f32, (p.r.y + p.r.h) as f32 - 10.0 * s);
                let end = (from.0 + (to.0 - from.0) * lk, from.1 + (to.1 - from.1) * lk);
                light_line(
                    px,
                    w,
                    from,
                    end,
                    LILAC,
                    (160.0 * lk) as u32,
                    (160.0 * lk) as u32,
                );
            }
        }
    }

    /// The status bar along the bottom: fills in blocks, then reads ALL
    /// SYSTEMS ONLINE.
    fn status(&self, px: &mut [u32], t: f32) {
        let (w, s) = (self.w, self.s);
        let k = (smoothstep(1.0, 1.2, t) * 256.0) as u32;
        if k == 0 {
            return;
        }
        let (x0, x1) = (w as f32 * 0.38, w as f32 * 0.62);
        let y = self.h as f32 * 0.94;
        let bh = (8.0 * s).max(2.0);
        if t < 2.6 {
            let p = smoothstep(1.2, 2.5, t);
            for (a, b) in [
                ((x0, y), (x1, y)),
                ((x0, y + bh), (x1, y + bh)),
                ((x0, y), (x0, y + bh)),
                ((x1, y), (x1, y + bh)),
            ] {
                light_line(px, w, a, b, LILAC, (k * 150) >> 8, (k * 150) >> 8);
            }
            let block = (8.0 * s).max(3.0);
            let mut x = x0 + 2.0;
            while x + block - 2.0 < x0 + 2.0 + (x1 - x0 - 4.0) * p {
                light_rect(
                    px,
                    w,
                    x as i32,
                    (y + 2.0) as i32,
                    (x + block - 2.0) as i32,
                    (y + bh - 1.0) as i32,
                    LILAC,
                    (k * 230) >> 8,
                );
                x += block;
            }
        } else {
            self.online.draw(px, w, neon(t, 2.6), true);
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
        // The view fades up out of the dark and the galaxy's glow with it.
        let glow = smoothstep(0.0, 1.2, t);
        let bright = smoothstep(0.0, 0.5, t);
        if glow >= 1.0 {
            px.copy_from_slice(&self.lit);
        } else {
            let (kg, kb) = ((glow * 256.0) as u32, (bright * 256.0) as u32);
            for ((p, &a), &b) in px.iter_mut().zip(&self.dark).zip(&self.lit) {
                *p = scale(over(a, b, kg), kb);
            }
        }

        // The galaxy, turning.
        let kf = (smoothstep(0.0, 1.0, t) * 256.0) as u32;
        for p in &self.particles {
            let (x, y) = self.place(p, t);
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

        self.hud(px, t);
        self.scan(px, t);
        for p in &self.panels {
            let ck = self.panel(px, p, t);
            if ck <= 0.0 {
                continue;
            }
            match p.kind {
                Kind::Radar => self.radar(px, p.r, t, ck),
                Kind::Spectrum => self.spectrum(px, p.r, t, ck),
                Kind::Nav => self.type_lines(px, &self.readouts, t, p.open + 0.3),
                Kind::Target => {
                    if t > 2.5 {
                        self.type_lines(px, &self.target, t, 2.5);
                    }
                }
            }
        }
        self.status(px, t);

        // The title: slashed on from the left with a hot edge, then its glow
        // strikes; then the version.
        if t > 2.4 {
            let wipe = smoothstep(2.4, 2.75, t);
            let upto = (wipe * (self.title_glow.w as f32 + 80.0)) as i32;
            let g = neon(t, 2.7);
            if g > 0 {
                self.title_glow.draw(px, w, (g * 210) >> 8, true);
            }
            self.title_edge.draw_upto(px, w, 256, false, upto);
            self.title.draw_upto(px, w, 256, false, upto - 24);
            if wipe < 1.0 {
                let x = (self.title_glow.x + upto) as f32;
                let (y0, y1) = (
                    self.title.y as f32 - 10.0,
                    (self.title.y + self.title_h) as f32 + 10.0,
                );
                for dx in -3i32..=3 {
                    let k = [40, 90, 180, 256, 180, 90, 40][(dx + 3) as usize];
                    light_line(
                        px,
                        w,
                        (x + dx as f32 + 30.0, y0),
                        (x + dx as f32 - 20.0, y1),
                        WHITE,
                        k,
                        k,
                    );
                }
            }
        }
        let k = neon(t, 3.0);
        if k > 0 {
            self.sub_glow.draw(px, w, (k * 220) >> 8, true);
            self.sub.draw(px, w, k, true);
        }
    }
}
