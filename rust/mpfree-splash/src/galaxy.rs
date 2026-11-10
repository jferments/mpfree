//! The startup scene: a star field, a spinning two-armed spiral galaxy, and
//! the words "MPCFree OS v0.1" in block letters in its glowing heart. The
//! galaxy is sized around the title: its arms start just outside the title
//! and wind outwards, so the whole galaxy turns around the words.
//!
//! The scene is drawn as a grid of square "pixels", two per character cell:
//! each cell is an upper half block (U+2580) whose foreground colour is the
//! top pixel and whose background colour is the bottom pixel. That works the
//! same in a desktop terminal and on the MPC's framebuffer backend.

use crate::font;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use std::f32::consts::{PI, TAU};

pub const TITLE: &str = "MPCFree OS v0.1";
/// The title split in two, for screens too narrow for one line.
const TITLE_LINES: [&str; 2] = ["MPCFree", "OS v0.1"];
/// One full turn of the galaxy takes this many seconds.
const TURN_SECONDS: f32 = 40.0;
/// The galaxy is seen tilted: its disc is squashed to this fraction vertically.
const TILT: f32 = 0.62;

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

struct Particle {
    /// Distance from the centre. Arm and disc stars: 0 is the edge of the
    /// heart and 1 the galaxy's rim. Heart stars: a fraction of the heart's radius.
    r: f32,
    heart: bool,
    /// Angle at time 0, in radians.
    angle: f32,
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

pub struct Galaxy {
    particles: Vec<Particle>,
    stars: Vec<Star>,
}

impl Default for Galaxy {
    fn default() -> Self {
        Galaxy::new()
    }
}

impl Galaxy {
    pub fn new() -> Galaxy {
        let mut rng = Rng(0x4d50_4346_7265_6521);
        let mut particles = Vec::new();
        // Two arms on a logarithmic spiral, with a scatter that grows outwards.
        for i in 0..6000 {
            let arm = (i % 2) as f32 * PI;
            let r = rng.unit().powf(1.25);
            let wind = 2.7 * r;
            let spread = (0.08 + 0.32 * r) * rng.gauss();
            let angle = arm + wind + spread;
            // Young blue stars on the arms, a pink glow on a few, older gold near the core.
            let t = rng.unit();
            let rgb = if t < 0.06 {
                [1.0, 0.35, 0.65]
            } else if r < 0.12 {
                [1.0, 0.82, 0.55]
            } else if t < 0.55 {
                [0.45, 0.62, 1.0]
            } else {
                [0.75, 0.82, 1.0]
            };
            let weight = (0.35 + 0.65 * rng.unit()) * (1.2 - r);
            particles.push(Particle { r, angle, rgb, weight, heart: false });
        }
        // Old gold stars swirling in the heart, behind the title.
        for _ in 0..900 {
            let r = (rng.gauss() * 0.45).abs().min(1.1);
            let angle = rng.unit() * TAU;
            particles.push(Particle { r, angle, rgb: [1.0, 0.8, 0.5], weight: 0.5 * rng.unit(), heart: true });
        }
        // A diffuse disc of dim stars between the arms.
        for _ in 0..1500 {
            let r = rng.unit().sqrt();
            let angle = rng.unit() * TAU;
            particles.push(Particle { r, angle, rgb: [0.6, 0.6, 0.85], weight: 0.12 * (1.1 - r), heart: false });
        }
        let stars = (0..260)
            .map(|_| Star {
                x: rng.unit(),
                y: rng.unit(),
                brightness: 0.25 + 0.75 * rng.unit().powi(3),
                twinkle: 0.6 + 2.4 * rng.unit(),
                phase: rng.unit() * TAU,
            })
            .collect();
        Galaxy { particles, stars }
    }

    /// The scene at `seconds` after start, as `width` x `height` pixels, row by row.
    pub fn pixels(&self, width: usize, height: usize, seconds: f32) -> Vec<[u8; 3]> {
        let mut acc = vec![[0f32; 3]; width * height];
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let (w, h) = (width as f32, height as f32);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let (heart, radius) = galaxy_size(width, height);

        // Background stars that twinkle.
        for s in &self.stars {
            let x = (s.x * w) as usize;
            let y = (s.y * h) as usize;
            if x < width && y < height {
                let b = s.brightness * (0.65 + 0.35 * (seconds * s.twinkle + s.phase).sin());
                for v in acc[y * width + x].iter_mut() {
                    *v += b * 1.6;
                }
            }
        }

        // The galaxy, turning as one (a density wave, so the arms keep their shape).
        let spin = -seconds / TURN_SECONDS * TAU;
        let gain = 1800.0 / (w * h).max(1.0) * 6.0;
        for p in &self.particles {
            let a = p.angle + spin;
            let r = if p.heart { p.r * heart } else { heart + p.r * (radius - heart) };
            let x = cx + a.cos() * r;
            let y = cy + a.sin() * r * TILT;
            splat(&mut acc, width, height, x, y, p.rgb, p.weight * gain);
        }

        // The glowing heart, a little wider than the title.
        let core = heart * 0.85;
        for py in 0..height {
            for px in 0..width {
                let dx = (px as f32 + 0.5 - cx) / core;
                let dy = (py as f32 + 0.5 - cy) / (core * TILT);
                let g = (-(dx * dx + dy * dy)).exp() * 1.3;
                if g > 0.002 {
                    let c = &mut acc[py * width + px];
                    c[0] += g;
                    c[1] += g * 0.88;
                    c[2] += g * 0.62;
                }
            }
        }

        let mut out: Vec<[u8; 3]> = acc
            .iter()
            .map(|c| {
                let tone = |v: f32| ((1.0 - (-v * 1.4).exp()) * 255.0).round().clamp(0.0, 255.0) as u8;
                // A faint deep-blue sky rather than pure black.
                [tone(c[0] + 0.012), tone(c[1] + 0.008), tone(c[2] + 0.035)]
            })
            .collect();
        draw_title(&mut out, width, height, seconds);
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

/// How the title is laid out: one line or two, and how many pixels per font pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleLayout {
    pub lines: usize,
    pub scale: usize,
}

impl TitleLayout {
    fn size(self) -> (usize, usize) {
        let chars = if self.lines == 1 { TITLE.len() } else { TITLE_LINES[0].len().max(TITLE_LINES[1].len()) };
        let w = chars * 8 * self.scale;
        let h = self.lines * 8 * self.scale + (self.lines - 1) * 2 * self.scale;
        (w, h)
    }
}

/// The biggest title that leaves room for the galaxy around it: at most 40%
/// of the screen's width and 30% of its height, one line winning a tie. On a
/// screen too small for that, the smallest title (two lines, 56x18 pixels).
pub fn title_layout(width: usize, height: usize) -> TitleLayout {
    let fits = |layout: TitleLayout| {
        let (w, h) = layout.size();
        w * 10 <= width * 4 && h * 10 <= height * 3
    };
    (1..=8)
        .rev()
        .flat_map(|scale| [1, 2].map(|lines| TitleLayout { lines, scale }))
        .find(|&l| fits(l))
        .unwrap_or(TitleLayout { lines: 2, scale: 1 })
}

/// The heart's radius (where the arms start) and the galaxy's radius, in
/// pixels along the long axis. The heart holds the title with a margin; the
/// galaxy fills the screen, or grows past it when the title is big for the screen.
pub fn galaxy_size(width: usize, height: usize) -> (f32, f32) {
    let (tw, th) = title_layout(width, height).size();
    let heart = (tw as f32 / 2.0 + 3.0).max((th as f32 / 2.0 + 3.0) / TILT);
    let fit = (width as f32 * 0.58).min(height as f32 * 0.6 / TILT);
    (heart, fit.max(heart * 2.2))
}

/// The block-letter pixels of the title: true where a letter is, row by row.
pub fn title_mask(layout: TitleLayout) -> (usize, usize, Vec<bool>) {
    let (w, h) = layout.size();
    let mut mask = vec![false; w * h];
    let lines: Vec<&str> = if layout.lines == 1 { vec![TITLE] } else { TITLE_LINES.to_vec() };
    for (li, line) in lines.iter().enumerate() {
        let line_w = line.len() * 8 * layout.scale;
        let left = (w - line_w) / 2;
        let top = li * 10 * layout.scale;
        for (ci, ch) in line.bytes().enumerate() {
            let glyph = font::GLYPHS[(ch - font::FIRST) as usize];
            for gy in 0..8 * layout.scale {
                for gx in 0..8 * layout.scale {
                    if glyph[gy / layout.scale] & (1 << (gx / layout.scale)) != 0 {
                        mask[(top + gy) * w + left + ci * 8 * layout.scale + gx] = true;
                    }
                }
            }
        }
    }
    (w, h, mask)
}

/// Draws the title in the centre: white to cyan letters with a dark outline,
/// and a slow shimmer moving across them.
fn draw_title(out: &mut [[u8; 3]], width: usize, height: usize, seconds: f32) {
    let layout = title_layout(width, height);
    let (tw, th, mask) = title_mask(layout);
    let left = (width as i64 - tw as i64) / 2;
    let top = (height as i64 - th as i64) / 2;
    let on = |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < tw && (y as usize) < th && mask[y as usize * tw + x as usize];
    let outline = layout.scale.div_ceil(2) as i64;
    for y in -outline..th as i64 + outline {
        for x in -outline..tw as i64 + outline {
            let (sx, sy) = (left + x, top + y);
            if sx < 0 || sy < 0 || sx as usize >= width || sy as usize >= height {
                continue;
            }
            let at = sy as usize * width + sx as usize;
            if on(x, y) {
                let v = y as f32 / th as f32;
                let shimmer = ((x as f32 / tw as f32 - seconds * 0.25).rem_euclid(1.0) - 0.5).abs();
                let glint = (1.0 - shimmer * 8.0).max(0.0) * 0.4;
                let mix = |a: f32, b: f32| ((a + (b - a) * v + glint * 255.0).min(255.0)) as u8;
                out[at] = [mix(255.0, 90.0), mix(255.0, 230.0), mix(255.0, 255.0)];
            } else {
                let near = (-outline..=outline).any(|dy| (-outline..=outline).any(|dx| on(x + dx, y + dy)));
                if near {
                    out[at] = [6, 4, 20];
                }
            }
        }
    }
}

/// A ratatui widget that draws the scene at a given time into any area.
pub struct GalaxyWidget<'a> {
    pub galaxy: &'a Galaxy,
    pub seconds: f32,
}

impl Widget for GalaxyWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (w, h) = (area.width as usize, area.height as usize * 2);
        let px = self.galaxy.pixels(w, h, self.seconds);
        if px.is_empty() {
            return;
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_exactly_what_jay_asked_for() {
        assert_eq!(TITLE, "MPCFree OS v0.1");
        assert_eq!(format!("{} {}", TITLE_LINES[0], TITLE_LINES[1]), TITLE);
    }

    #[test]
    fn title_layouts() {
        // The MPC One in landscape, 8x16-pixel cells: 160x50 cells, 160x100 pixels.
        assert_eq!(title_layout(160, 100), TitleLayout { lines: 2, scale: 1 });
        // An 80x24 terminal is too small to leave room, so the smallest title.
        assert_eq!(title_layout(80, 48), TitleLayout { lines: 2, scale: 1 });
        // A wide, short terminal gets one line.
        assert_eq!(title_layout(400, 100), TitleLayout { lines: 1, scale: 1 });
    }

    #[test]
    fn the_title_fits_inside_the_heart() {
        for (w, h) in [(160, 100), (80, 48), (320, 120), (40, 20), (1000, 600)] {
            let (heart, radius) = galaxy_size(w, h);
            let (tw, th) = title_layout(w, h).size();
            assert!(heart >= tw as f32 / 2.0 + 2.0, "{w}x{h}: heart {heart}, title {tw}");
            assert!(heart * TILT >= th as f32 / 2.0 + 2.0, "{w}x{h}: heart {heart}, title {th}");
            assert!(radius >= heart * 2.0, "{w}x{h}: arms need room");
        }
    }

    #[test]
    fn title_mask_has_letters() {
        let (w, h, mask) = title_mask(TitleLayout { lines: 1, scale: 1 });
        assert_eq!((w, h), (120, 8));
        // 'M' row 0 is 0x63: pixels 0, 1, 5 and 6 are set.
        assert!(mask[0] && mask[1] && !mask[2] && mask[5] && mask[6]);
        // The space between "MPCFree" and "OS" (characters 7) is empty.
        assert!((0..8).all(|y| (56..64).all(|x| !mask[y * w + x])));
    }

    #[test]
    fn scene_is_deterministic_and_turns() {
        let g = Galaxy::new();
        let a = g.pixels(160, 100, 0.0);
        assert_eq!(a, Galaxy::new().pixels(160, 100, 0.0));
        let b = g.pixels(160, 100, 5.0);
        assert_ne!(a, b, "the galaxy should move");
        let bright = |p: [u8; 3]| p.iter().map(|&v| v as u32).sum::<u32>();
        // The heart glows just above the title (two lines, 56x18 from (52, 41)).
        assert!(bright(a[39 * 160 + 80]) > 250, "{:?}", a[39 * 160 + 80]);
        assert!(bright(a[0]) < 200, "{:?}", a[0]);
        // 'M' starts with a lit pixel, with the dark outline just left of it.
        assert!(a[41 * 160 + 52][0] > 200, "{:?}", a[41 * 160 + 52]);
        assert_eq!(a[41 * 160 + 51], [6, 4, 20]);
    }

    #[test]
    fn widget_fills_every_cell() {
        let g = Galaxy::new();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        GalaxyWidget { galaxy: &g, seconds: 1.0 }.render(area, &mut buf);
        for y in 0..24 {
            for x in 0..80 {
                assert_eq!(buf[(x, y)].symbol(), "\u{2580}");
            }
        }
    }
}
