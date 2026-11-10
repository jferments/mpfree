//! The startup scene: a star field and a four-armed spiral galaxy, like the
//! Milky Way (two bright major arms and two fainter minor ones), turning
//! around the words "MPCFree OS v0.1" in its glowing heart.
//!
//! The galaxy is sized around the title: the heart holds the words and the
//! arms start just outside them and wind outwards. Each frame is a smooth
//! glow (an exponential disc lit by the four arms, computed per pixel) with
//! about 14,000 stars on top for sparkle.
//!
//! The scene is drawn as a grid of square "pixels", two per character cell:
//! each cell is an upper half block (U+2580) whose foreground colour is the
//! top pixel and whose background colour is the bottom pixel. That works the
//! same in a desktop terminal and on the MPC's framebuffer backend.

use crate::title::{self, TitleFont, TitleMask};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use crate::menu;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;
use std::cell::RefCell;
use std::f32::consts::{FRAC_PI_2, TAU};

pub const TITLE: &str = "MPCFree OS v0.1";
/// The title split in two, for screens too narrow for one line.
const TITLE_LINES: [&str; 2] = ["MPCFree", "OS v0.1"];
/// One full turn of the galaxy takes this many seconds.
pub const TURN_SECONDS: f32 = 45.0;
/// The galaxy is seen tilted: its disc is squashed to this fraction vertically.
pub const TILT: f32 = 0.62;
/// Arm pitch: how tightly the arms wind (1 / tan of the pitch angle, ~15 degrees).
pub const WIND: f32 = 3.6;
/// The four arms, a quarter turn apart: brightness of each.
pub const ARMS: [f32; 4] = [1.0, 0.55, 1.0, 0.55];
/// The heart's stars turn this much faster than the disc.
pub const HEART_SPEED: f32 = 1.7;

/// A small, fixed pseudo-random generator, so every run draws the same sky.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Roughly normal, mean 0, standard deviation 1.
    fn gauss(&mut self) -> f32 {
        (0..4).map(|_| self.unit()).sum::<f32>() * 1.732 - 3.464
    }
}

#[derive(Clone, Copy)]
enum Place {
    /// On the disc: `rn` 0 is the edge of the heart, 1 the rim; `offset` is the
    /// angle from the arm's centre line.
    Disc { rn: f32, offset: f32 },
    /// In the heart: distance as a fraction of the heart's radius, and angle.
    Heart { r: f32, angle: f32 },
}

struct Particle {
    place: Place,
    rgb: [f32; 3],
    weight: f32,
}

struct Star {
    /// Position as fractions of the screen's width and height.
    x: f32,
    y: f32,
    brightness: f32,
    twinkle: f32,
    phase: f32,
}

/// Everything that depends only on the screen size, worked out once.
struct Layout {
    width: usize,
    height: usize,
    title: TitleMask,
    title_left: i64,
    title_top: i64,
    /// Per pixel: brightness of the disc, distance out (0 heart, 1 rim), and the arm phase at spin 0.
    disc: Vec<f32>,
    out: Vec<f32>,
    arm_phase: Vec<f32>,
    /// Per pixel: brightness of the heart's glow.
    glow: Vec<f32>,
    /// Per particle: position in the disc's own plane at spin 0, and whether it is in the heart.
    points: Vec<(f32, f32, bool)>,
}

pub struct Galaxy {
    particles: Vec<Particle>,
    stars: Vec<Star>,
    font: TitleFont,
    layout: RefCell<Option<Layout>>,
}

impl Default for Galaxy {
    fn default() -> Self {
        Galaxy::new()
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Width (radians) of an arm at `rn`: arms widen outwards.
fn arm_width(rn: f32) -> f32 {
    0.16 + 0.2 * rn
}

impl Galaxy {
    /// The galaxy with the built-in block letters.
    pub fn new() -> Galaxy {
        Galaxy::with_font(TitleFont::Block)
    }

    /// The galaxy with the title in `font`.
    pub fn with_font(font: TitleFont) -> Galaxy {
        let mut rng = Rng(0x4d50_4346_7265_6521);
        let mut particles = Vec::new();
        // Stars along the arms: young blue and white ones, pink star-forming knots.
        for i in 0..11000 {
            let arm = i % 4;
            if ARMS[arm] < 1.0 && rng.unit() > ARMS[arm] + 0.15 {
                continue;
            }
            let rn = rng.unit().powf(1.3);
            let offset = arm as f32 * FRAC_PI_2 + rng.gauss() * arm_width(rn) * 0.8;
            let t = rng.unit();
            let rgb = if t < 0.05 {
                [1.0, 0.35, 0.7]
            } else if rn < 0.1 {
                [1.0, 0.85, 0.6]
            } else if t < 0.6 {
                [0.55, 0.7, 1.0]
            } else {
                [0.9, 0.92, 1.0]
            };
            let weight = (0.15 + 0.6 * rng.unit().powi(2)) * (1.25 - rn);
            particles.push(Particle { place: Place::Disc { rn, offset }, rgb, weight });
        }
        // Older, dimmer stars everywhere on the disc.
        for _ in 0..4000 {
            let rn = rng.unit().powf(1.5);
            let offset = rng.unit() * TAU;
            let weight = 0.12 + 0.2 * rng.unit();
            particles.push(Particle { place: Place::Disc { rn, offset }, rgb: [0.8, 0.75, 0.75], weight });
        }
        // Gold stars swirling in the heart, behind the title.
        for _ in 0..1800 {
            let r = (rng.gauss() * 0.5).abs().min(1.2);
            let angle = rng.unit() * TAU;
            particles.push(Particle { place: Place::Heart { r, angle }, rgb: [1.0, 0.8, 0.5], weight: 0.4 * rng.unit() });
        }
        let stars = (0..420)
            .map(|_| Star {
                x: rng.unit(),
                y: rng.unit(),
                brightness: 0.25 + 0.75 * rng.unit().powi(3),
                twinkle: 0.6 + 2.4 * rng.unit(),
                phase: rng.unit() * TAU,
            })
            .collect();
        Galaxy { particles, stars, font, layout: RefCell::new(None) }
    }

    fn layout(&self, width: usize, height: usize) -> Layout {
        let (lines, cap) = title_layout(&self.font, width, height);
        let title = title::rasterise(&self.font, &lines, cap);
        let (heart, radius) = galaxy_size(&self.font, width, height);
        let (cx, cy) = (width as f32 / 2.0, height as f32 / 2.0);
        let span = radius - heart;
        let mut disc = vec![0.0; width * height];
        let mut out = vec![0.0; width * height];
        let mut arm_phase = vec![0.0; width * height];
        let mut glow = vec![0.0; width * height];
        for py in 0..height {
            for px in 0..width {
                let i = py * width + px;
                let dx = px as f32 + 0.5 - cx;
                let dy = (py as f32 + 0.5 - cy) / TILT;
                let r = (dx * dx + dy * dy).sqrt();
                let rn = (r - heart) / span;
                out[i] = rn.clamp(0.0, 1.0);
                // Bright inside, fading out to the rim; the arms start at the heart's edge.
                disc[i] = (-rn.max(0.0) * 2.0).exp()
                    * smoothstep(heart * 0.75, heart * 1.1, r)
                    * (1.0 - smoothstep(0.85, 1.15, rn));
                arm_phase[i] = dy.atan2(dx) - WIND * (1.0 + rn.max(0.0) * (radius / heart - 1.0)).ln();
                let g = r / (heart * 0.95);
                glow[i] = (-g * g).exp() * 1.25;
            }
        }
        let points = self
            .particles
            .iter()
            .map(|p| match p.place {
                Place::Disc { rn, offset } => {
                    let r = heart + rn * span;
                    let a = offset + WIND * (r / heart).ln();
                    (r * a.cos(), r * a.sin(), false)
                }
                Place::Heart { r, angle } => (r * heart * angle.cos(), r * heart * angle.sin(), true),
            })
            .collect();
        Layout {
            width,
            height,
            title_left: (width as i64 - title.width as i64) / 2,
            title_top: (height as i64 - title.height as i64) / 2,
            title,
            disc,
            out,
            arm_phase,
            glow,
            points,
        }
    }

    /// The scene at `seconds` after start, as `width` x `height` pixels, row by row.
    pub fn pixels(&self, width: usize, height: usize, seconds: f32) -> Vec<[u8; 3]> {
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let mut cached = self.layout.borrow_mut();
        if !matches!(&*cached, Some(l) if l.width == width && l.height == height) {
            *cached = Some(self.layout(width, height));
        }
        let l = cached.as_ref().unwrap();
        let (w, h) = (width as f32, height as f32);
        let (cx, cy) = (w / 2.0, h / 2.0);
        // Arms trail: the galaxy turns against the direction the arms wind.
        let spin = -seconds / TURN_SECONDS * TAU;

        // The smooth glow: disc, four arms and the heart.
        let mut acc = vec![[0f32; 3]; width * height];
        for (i, c) in acc.iter_mut().enumerate() {
            let disc = l.disc[i];
            if disc > 0.003 {
                let psi = l.arm_phase[i] - spin;
                let k = (psi / FRAC_PI_2).round();
                let d = psi - k * FRAC_PI_2;
                let width = arm_width(l.out[i]) * 1.15;
                // A smooth bump across the arm, close to exp(-q) and cheaper.
                let q = (d / width) * (d / width);
                let bump = if q < 4.0 { (1.0 - q * 0.25).powi(4) } else { 0.0 };
                let arm = ARMS[(k as i64).rem_euclid(4) as usize] * bump;
                let v = disc * 0.95;
                c[0] += v * (0.10 + 0.42 * arm);
                c[1] += v * (0.09 + 0.52 * arm);
                c[2] += v * (0.14 + 0.85 * arm);
            }
            let g = l.glow[i];
            c[0] += g;
            c[1] += g * 0.86;
            c[2] += g * 0.6;
        }

        // Background stars that twinkle.
        for s in &self.stars {
            let x = (s.x * w) as usize;
            let y = (s.y * h) as usize;
            if x < width && y < height {
                let b = s.brightness * (0.65 + 0.35 * (seconds * s.twinkle + s.phase).sin());
                for v in acc[y * width + x].iter_mut() {
                    *v += b * 1.4;
                }
            }
        }

        // Stars of the galaxy, turning with it (the heart a little faster).
        let disc_turn = spin.sin_cos();
        let heart_turn = (spin * HEART_SPEED).sin_cos();
        for (p, &(x0, y0, heart)) in self.particles.iter().zip(&l.points) {
            let (s, c) = if heart { heart_turn } else { disc_turn };
            let x = cx + x0 * c - y0 * s;
            let y = cy + (x0 * s + y0 * c) * TILT;
            splat(&mut acc, width, height, x, y, p.rgb, p.weight);
        }

        let mut out: Vec<[u8; 3]> = acc
            .iter()
            .map(|c| {
                let tone = |v: f32| ((1.0 - (-v * 1.4).exp()) * 255.0).round().clamp(0.0, 255.0) as u8;
                // A faint deep-blue sky rather than pure black.
                [tone(c[0] + 0.012), tone(c[1] + 0.008), tone(c[2] + 0.035)]
            })
            .collect();
        draw_title(&mut out, l, seconds);
        out
    }
}

/// Adds a point of light at (x, y), shared among the four nearest pixels.
fn splat(acc: &mut [[f32; 3]], width: usize, height: usize, x: f32, y: f32, rgb: [f32; 3], weight: f32) {
    let (x, y) = (x - 0.5, y - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    for (dx, dy, f) in [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)] {
        let (px, py) = (x0 as i64 + dx, y0 as i64 + dy);
        if px >= 0 && py >= 0 && (px as usize) < width && (py as usize) < height {
            let c = &mut acc[py as usize * width + px as usize];
            for k in 0..3 {
                c[k] += rgb[k] * weight * f;
            }
        }
    }
}

/// The title's lines and its cap height in pixels: as big as fits in at most
/// 40% of the screen's width and 30% of its height, on one line or two,
/// whichever is bigger. Never smaller than 6 pixels.
pub fn title_layout(font: &TitleFont, width: usize, height: usize) -> (Vec<&'static str>, f32) {
    let fit = |lines: &[&str]| {
        let mut cap = 80.0;
        while cap > 6.0 {
            let (w, h) = title::size(font, lines, cap);
            if w * 10 <= width * 4 && h * 10 <= height * 3 {
                break;
            }
            cap -= 0.5;
        }
        cap
    };
    let one = fit(&[TITLE]);
    let two = fit(&TITLE_LINES);
    if one >= two { (vec![TITLE], one) } else { (TITLE_LINES.to_vec(), two) }
}

/// The heart's radius (where the arms start) and the galaxy's radius, in
/// pixels along the long axis. The heart holds the title with a margin; the
/// galaxy fills the screen, or grows past it when the title is big for the screen.
pub fn galaxy_size(font: &TitleFont, width: usize, height: usize) -> (f32, f32) {
    let (lines, cap) = title_layout(font, width, height);
    let (tw, th) = title::size(font, &lines, cap);
    let heart = (tw as f32 / 2.0 + 2.0).max((th as f32 / 2.0 + 2.0) / TILT);
    let fit = (width as f32 * 0.56).min(height as f32 * 0.6 / TILT);
    (heart, fit.max(heart * 2.4))
}

/// Draws the title in the centre: white to cyan letters with a dark outline,
/// and a slow shimmer moving across them.
fn draw_title(out: &mut [[u8; 3]], l: &Layout, seconds: f32) {
    let t = &l.title;
    for y in 0..t.height {
        for x in 0..t.width {
            let i = y * t.width + x;
            let (halo, ink) = (t.halo[i], t.ink[i]);
            if halo <= 0.0 {
                continue;
            }
            let (sx, sy) = (l.title_left + x as i64, l.title_top + y as i64);
            if sx < 0 || sy < 0 || sx as usize >= l.width || sy as usize >= l.height {
                continue;
            }
            let at = sy as usize * l.width + sx as usize;
            let v = y as f32 / t.height as f32;
            let shimmer = ((x as f32 / t.width as f32 - seconds * 0.22).rem_euclid(1.0) - 0.5).abs();
            let glint = (1.0 - shimmer * 8.0).max(0.0) * 0.35 * 255.0;
            let letter = [255.0 + (90.0 - 255.0) * v + glint, 255.0 + (225.0 - 255.0) * v + glint, 255.0];
            let outline = [6.0, 4.0, 20.0];
            let p = &mut out[at];
            for k in 0..3 {
                let under = p[k] as f32 * (1.0 - halo) + outline[k] * halo;
                p[k] = (under * (1.0 - ink) + letter[k].min(255.0) * ink).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// A ratatui widget that draws the scene at a given time into any area.
pub struct GalaxyWidget<'a> {
    pub galaxy: &'a Galaxy,
    pub seconds: f32,
    /// The boot menu to draw over the scene, if any.
    pub menu: Option<&'a menu::MenuText>,
}

impl Widget for GalaxyWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (w, h) = (area.width as usize, area.height as usize * 2);
        let mut px = self.galaxy.pixels(w, h, self.seconds);
        if px.is_empty() {
            return;
        }
        let pixel_menu = menu::fits_pixels(w, h);
        if let (Some(m), true) = (self.menu, pixel_menu) {
            menu::draw_pixels(&mut px, w, h, m);
        }
        for row in 0..area.height as usize {
            for col in 0..w {
                let top = px[(row * 2) * w + col];
                let bottom = px[(row * 2 + 1) * w + col];
                if let Some(cell) = buf.cell_mut((area.x + col as u16, area.y + row as u16)) {
                    cell.set_symbol("\u{2580}")
                        .set_fg(Color::Rgb(top[0], top[1], top[2]))
                        .set_bg(Color::Rgb(bottom[0], bottom[1], bottom[2]));
                }
            }
        }
        if let (Some(m), false) = (self.menu, pixel_menu) {
            // Too small for the pixel font: plain text near the bottom.
            let mut lines: Vec<(String, Style)> = Vec::new();
            let options: Vec<String> =
                m.options.iter().map(|(l, on)| if *on { format!("[ {l} ]") } else { format!("  {l}  ") }).collect();
            lines.push((options.join("  "), Style::new().fg(Color::Rgb(90, 225, 255)).bg(Color::Rgb(5, 10, 30))));
            for hint in &m.hints {
                lines.push((hint.clone(), Style::new().fg(Color::Rgb(230, 230, 240)).bg(Color::Rgb(5, 10, 30))));
            }
            let top = area.y + area.height.saturating_sub(lines.len() as u16 + 1);
            for (i, (text, style)) in lines.iter().enumerate() {
                let len = text.chars().count() as u16;
                let x = area.x + area.width.saturating_sub(len) / 2;
                buf.set_stringn(x, top + i as u16, text, area.width as usize, *style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn title_is_exactly_what_jay_asked_for() {
        assert_eq!(TITLE, "MPCFree OS v0.1");
        assert_eq!(format!("{} {}", TITLE_LINES[0], TITLE_LINES[1]), TITLE);
    }

    #[test]
    fn title_layouts() {
        // The MPC One at 4x8-pixel cells: 320x100 cells, 320x200 pixels. Two lines.
        let (lines, cap) = title_layout(&TitleFont::Block, 320, 200);
        assert_eq!(lines, TITLE_LINES);
        assert!(cap >= 18.0, "{cap}");
        // A wide, short terminal gets one line.
        assert_eq!(title_layout(&TitleFont::Block, 400, 100).0, [TITLE]);
        // A tiny one still gets a title.
        assert_eq!(title_layout(&TitleFont::Block, 40, 20).1, 6.0);
    }

    #[test]
    fn the_title_fits_inside_the_heart() {
        for (w, h) in [(320, 200), (160, 100), (80, 48), (250, 128), (40, 20), (1000, 600)] {
            let font = TitleFont::Block;
            let (heart, radius) = galaxy_size(&font, w, h);
            let (lines, cap) = title_layout(&font, w, h);
            let (tw, th) = title::size(&font, &lines, cap);
            assert!(heart >= tw as f32 / 2.0 + 1.0, "{w}x{h}: heart {heart}, title {tw}");
            assert!(heart * TILT >= th as f32 / 2.0 + 1.0, "{w}x{h}: heart {heart}, title {th}");
            assert!(radius >= heart * 2.0, "{w}x{h}: arms need room");
        }
    }

    #[test]
    fn scene_is_deterministic_and_turns() {
        let g = Galaxy::new();
        let a = g.pixels(320, 200, 0.0);
        assert_eq!(a, Galaxy::new().pixels(320, 200, 0.0));
        let b = g.pixels(320, 200, 5.0);
        assert_ne!(a, b, "the galaxy should move");
        let bright = |p: [u8; 3]| p.iter().map(|&v| v as u32).sum::<u32>();
        // The far corner is dark sky; the centre of the title is letters or the heart's glow.
        assert!(bright(a[0]) < 200, "{:?}", a[0]);
        assert!(bright(a[100 * 320 + 160]) > 250, "{:?}", a[100 * 320 + 160]);
        // The disc is lit well outside the heart: no sparse, empty galaxy.
        let (heart, radius) = galaxy_size(&TitleFont::Block, 320, 200);
        let lit = (0..360)
            .filter(|deg| {
                let t = *deg as f32 * PI / 180.0;
                let r = heart + (radius - heart) * 0.3;
                let (x, y) = (160.0 + r * t.cos(), 100.0 + r * t.sin() * TILT);
                bright(a[y as usize * 320 + x as usize]) > 60
            })
            .count();
        assert!(lit > 120, "only {lit} of 360 directions are lit at 30% out");
    }

    #[test]
    fn widget_fills_every_cell() {
        let g = Galaxy::new();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        GalaxyWidget { galaxy: &g, seconds: 1.0, menu: None }.render(area, &mut buf);
        for y in 0..24 {
            for x in 0..80 {
                assert_eq!(buf[(x, y)].symbol(), "\u{2580}");
            }
        }
    }
}
