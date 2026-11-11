//! Text layout and rendering for the MPFree test boot's on-screen log.
//!
//! This library does no I/O. It wraps log lines to the screen's width and
//! renders one frame in the framebuffer's own pixel layout: the size, line
//! stride and pixel format that the kernel reports for /dev/fb0, and the
//! panel's mounting rotation from the device tree. `main.rs` does the I/O.
#![forbid(unsafe_code)]

use std::collections::VecDeque;

pub mod font;

/// Each 8x8 glyph is drawn 2x2 pixels per font pixel.
pub const SCALE: usize = 2;
pub const CELL_W: usize = 8 * SCALE;
/// Two blank pixel rows between text lines.
pub const CELL_H: usize = 8 * SCALE + 2;
const MAX_SIDE: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32 bits per pixel, stored B, G, R, X (little-endian XRGB8888).
    Xrgb8888,
    /// 16 bits per pixel, little-endian RGB565.
    Rgb565,
}

/// How the panel is mounted, as the device tree's panel `rotation` property
/// says (degrees counter-clockwise; Linux calls 270 "left side up").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl Rotation {
    pub fn from_degrees(degrees: u32) -> Option<Rotation> {
        match degrees {
            0 => Some(Rotation::Deg0),
            90 => Some(Rotation::Deg90),
            180 => Some(Rotation::Deg180),
            270 => Some(Rotation::Deg270),
            _ => None,
        }
    }

    fn quarter(self) -> bool {
        matches!(self, Rotation::Deg90 | Rotation::Deg270)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Visible size in the framebuffer's own (native) pixels.
    pub width: usize,
    pub height: usize,
    /// Bytes from one native row to the next.
    pub stride: usize,
    pub format: PixelFormat,
}

impl Geometry {
    pub fn new(width: usize, height: usize, stride: usize, bits_per_pixel: u32) -> Result<Geometry, String> {
        let format = match bits_per_pixel {
            32 => PixelFormat::Xrgb8888,
            16 => PixelFormat::Rgb565,
            other => return Err(format!("unsupported pixel depth {other} bits")),
        };
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(format!("implausible screen size {width}x{height}"));
        }
        let bytes = if format == PixelFormat::Xrgb8888 { 4 } else { 2 };
        if stride < width * bytes || stride > MAX_SIDE * 4 {
            return Err(format!("stride {stride} does not fit {width} pixels of {bytes} bytes"));
        }
        Ok(Geometry { width, height, stride, format })
    }

    pub fn bytes_per_pixel(&self) -> usize {
        if self.format == PixelFormat::Xrgb8888 { 4 } else { 2 }
    }

    /// Bytes of one visible screen: what one frame writes, from offset 0.
    pub fn frame_len(&self) -> usize {
        self.stride * self.height
    }

    /// The size as the viewer sees it, after the panel's rotation.
    pub fn logical_size(&self, rotation: Rotation) -> (usize, usize) {
        if rotation.quarter() { (self.height, self.width) } else { (self.width, self.height) }
    }

    /// Native (x, y) of the logical pixel (lx, ly), where (0, 0) is the
    /// top left corner as the viewer sees it.
    pub fn native(&self, rotation: Rotation, lx: usize, ly: usize) -> (usize, usize) {
        let (w, h) = (self.width, self.height);
        match rotation {
            Rotation::Deg0 => (lx, ly),
            // Right side up: the panel's right edge is at the top.
            Rotation::Deg90 => (w - 1 - ly, lx),
            Rotation::Deg180 => (w - 1 - lx, h - 1 - ly),
            // Left side up: the panel's left edge is at the top.
            Rotation::Deg270 => (ly, h - 1 - lx),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub const BACKGROUND: Rgb = Rgb(0, 0, 0);
pub const TEXT: Rgb = Rgb(200, 200, 200);
pub const TITLE_FG: Rgb = Rgb(255, 255, 255);
pub const TITLE_BG: Rgb = Rgb(20, 60, 160);
pub const STATUS_FG: Rgb = Rgb(0, 0, 0);
pub const STATUS_BG: Rgb = Rgb(230, 200, 40);

/// Printable ASCII only: tabs become spaces, everything else becomes '?'.
pub fn sanitize(line: &str) -> String {
    line.trim_end_matches(['\r', '\n'])
        .chars()
        .map(|c| match c {
            '\t' => ' ',
            ' '..='~' => c,
            _ => '?',
        })
        .collect()
}

/// The newest wrapped text rows, as many as fit below the title and status rows.
pub struct LogView {
    cols: usize,
    rows: usize,
    lines: VecDeque<String>,
}

impl LogView {
    pub fn new(cols: usize, rows: usize) -> LogView {
        LogView { cols: cols.max(8), rows: rows.max(1), lines: VecDeque::new() }
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Adds one log line, wrapped; continuation rows are indented by two spaces.
    pub fn push(&mut self, line: &str) {
        let text: Vec<char> = sanitize(line).chars().collect();
        let mut start = 0;
        let mut first = true;
        loop {
            let room = if first { self.cols } else { self.cols - 2 };
            let end = (start + room).min(text.len());
            let piece: String = text[start..end].iter().collect();
            self.lines.push_back(if first { piece } else { format!("  {piece}") });
            while self.lines.len() > self.rows {
                self.lines.pop_front();
            }
            start = end;
            first = false;
            if start >= text.len() {
                break;
            }
        }
    }

    pub fn rows(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(|s| s.as_str())
    }
}

/// Text rows and columns that fit the screen as the viewer sees it.
pub fn grid(geometry: &Geometry, rotation: Rotation) -> (usize, usize) {
    let (lw, lh) = geometry.logical_size(rotation);
    (lw / CELL_W, lh / CELL_H)
}

struct Painter<'a> {
    geometry: Geometry,
    rotation: Rotation,
    buf: &'a mut [u8],
}

impl Painter<'_> {
    fn pixel(&mut self, lx: usize, ly: usize, color: Rgb) {
        let (x, y) = self.geometry.native(self.rotation, lx, ly);
        let at = y * self.geometry.stride + x * self.geometry.bytes_per_pixel();
        match self.geometry.format {
            PixelFormat::Xrgb8888 => self.buf[at..at + 4].copy_from_slice(&[color.2, color.1, color.0, 0]),
            PixelFormat::Rgb565 => {
                let v = ((color.0 as u16 >> 3) << 11) | ((color.1 as u16 >> 2) << 5) | (color.2 as u16 >> 3);
                self.buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
    }

    fn row(&mut self, row: usize, text: &str, fg: Rgb, bg: Rgb, cols: usize) {
        let chars: Vec<u8> = text.bytes().collect();
        for col in 0..cols {
            let glyph = chars
                .get(col)
                .filter(|c| (font::FIRST..=font::LAST).contains(c))
                .map(|c| font::GLYPHS[(c - font::FIRST) as usize])
                .unwrap_or([0; 8]);
            for py in 0..CELL_H {
                let bits = if py < 8 * SCALE { glyph[py / SCALE] } else { 0 };
                for px in 0..CELL_W {
                    let on = bits & (1 << (px / SCALE)) != 0;
                    self.pixel(col * CELL_W + px, row * CELL_H + py, if on { fg } else { bg });
                }
            }
        }
    }
}

/// Renders one frame into `buf` (`geometry.frame_len()` bytes): a title row,
/// a status row, then the log rows, oldest at the top.
pub fn render(geometry: &Geometry, rotation: Rotation, title: &str, status: &str, view: &LogView,
              buf: &mut [u8]) -> Result<(), String> {
    if buf.len() != geometry.frame_len() {
        return Err(format!("frame buffer is {} bytes, expected {}", buf.len(), geometry.frame_len()));
    }
    let (cols, rows) = grid(geometry, rotation);
    if cols < 8 || rows < 3 {
        return Err("the screen is too small for the log".to_string());
    }
    buf.fill(0);
    let mut painter = Painter { geometry: *geometry, rotation, buf };
    painter.row(0, &sanitize(title), TITLE_FG, TITLE_BG, cols);
    painter.row(1, &sanitize(status), STATUS_FG, STATUS_BG, cols);
    for (i, line) in view.rows().take(rows - 2).enumerate() {
        painter.row(i + 2, line, TEXT, BACKGROUND, cols);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> Geometry {
        // As the MPC One reports it: an 800x1280 panel, 32 bits per pixel, stride 3200.
        Geometry::new(800, 1280, 3200, 32).unwrap()
    }

    #[test]
    fn geometry_is_checked() {
        assert!(Geometry::new(800, 1280, 3199, 32).is_err());
        assert!(Geometry::new(800, 1280, 3200, 24).is_err());
        assert!(Geometry::new(0, 1280, 3200, 32).is_err());
        let g = Geometry::new(800, 1280, 1600, 16).unwrap();
        assert_eq!((g.format, g.frame_len()), (PixelFormat::Rgb565, 1600 * 1280));
    }

    #[test]
    fn left_side_up_maps_corners() {
        // rotation = 270 in Akai's device tree: the viewer sees 1280x800, with the
        // panel's left edge (native x = 0) at the top and native y = 0 on the right.
        let g = panel();
        let r = Rotation::from_degrees(270).unwrap();
        assert_eq!(g.logical_size(r), (1280, 800));
        assert_eq!(g.native(r, 0, 0), (0, 1279), "top left");
        assert_eq!(g.native(r, 1279, 0), (0, 0), "top right");
        assert_eq!(g.native(r, 0, 799), (799, 1279), "bottom left");
        assert_eq!(g.native(r, 1279, 799), (799, 0), "bottom right");
        assert_eq!(grid(&g, r), (80, 44));
    }

    #[test]
    fn every_rotation_stays_inside_the_panel_and_is_one_to_one() {
        let g = Geometry::new(6, 4, 24, 32).unwrap();
        for degrees in [0, 90, 180, 270] {
            let r = Rotation::from_degrees(degrees).unwrap();
            let (lw, lh) = g.logical_size(r);
            let mut seen = [false; 24];
            for ly in 0..lh {
                for lx in 0..lw {
                    let (x, y) = g.native(r, lx, ly);
                    assert!(x < 6 && y < 4, "{degrees}: ({lx},{ly}) -> ({x},{y})");
                    assert!(!seen[y * 6 + x], "{degrees}: two pixels land on ({x},{y})");
                    seen[y * 6 + x] = true;
                }
            }
        }
        assert_eq!(Rotation::from_degrees(45), None);
    }

    #[test]
    fn wraps_and_keeps_the_newest_rows() {
        let mut v = LogView::new(10, 3);
        v.push("0123456789abcdefghij\n");
        assert_eq!(v.rows().collect::<Vec<_>>(), ["0123456789", "  abcdefgh", "  ij"]);
        v.push("tab\there\u{e9}");
        assert_eq!(v.rows().collect::<Vec<_>>(), ["  abcdefgh", "  ij", "tab here?"]);
        v.push("");
        assert_eq!(v.rows().last(), Some(""));
    }

    #[test]
    fn renders_glyphs_where_the_viewer_reads_them() {
        let g = panel();
        let r = Rotation::Deg270;
        let mut view = LogView::new(80, 42);
        view.push("A");
        let mut buf = vec![0u8; g.frame_len()];
        render(&g, r, "T", "S", &view, &mut buf).unwrap();
        let at = |lx: usize, ly: usize| {
            let (x, y) = g.native(r, lx, ly);
            let o = y * g.stride + x * 4;
            [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]
        };
        // The title row's background is blue, stored B, G, R, X.
        assert_eq!(at(CELL_W * 50, 1), [160, 60, 20, 0]);
        // "A" in font8x8: row 0 is 0x0c, so font pixels 2 and 3 are set and 0 and 1 are not.
        let (y0, x0) = (2 * CELL_H, 0);
        assert_eq!(at(x0 + 2 * SCALE, y0), [200, 200, 200, 0]);
        assert_eq!(at(x0 + 3 * SCALE + 1, y0 + 1), [200, 200, 200, 0]);
        assert_eq!(at(x0, y0), [0, 0, 0, 0]);
        // Row 4 of "A" is 0x3f: the bar across.
        assert_eq!(at(x0 + 5 * SCALE, y0 + 4 * SCALE), [200, 200, 200, 0]);
        // The spacing rows under a text row stay background.
        assert_eq!(at(x0 + 2 * SCALE, y0 + 8 * SCALE), [0, 0, 0, 0]);
        let mut small = vec![0u8; 10];
        assert!(render(&g, r, "T", "S", &view, &mut small).is_err());
    }

    #[test]
    fn rgb565_pixels() {
        let g = Geometry::new(800, 1280, 1600, 16).unwrap();
        let mut buf = vec![0u8; g.frame_len()];
        render(&g, Rotation::Deg270, "T", "S", &LogView::new(80, 42), &mut buf).unwrap();
        let (x, y) = g.native(Rotation::Deg270, CELL_W * 50, 1);
        let o = y * g.stride + x * 2;
        let v = u16::from_le_bytes([buf[o], buf[o + 1]]);
        assert_eq!(v, ((20u16 >> 3) << 11) | ((60u16 >> 2) << 5) | (160u16 >> 3));
    }
}
