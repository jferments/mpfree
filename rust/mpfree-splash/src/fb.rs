//! A ratatui backend that draws character cells straight into a Linux
//! framebuffer's pixel layout, for screens with no text console (both of
//! Akai's MPC One kernels have `CONFIG_FRAMEBUFFER_CONSOLE` off).
//!
//! The geometry and rotation code follows `mpfree-screen-log` (kexec test
//! boot), which drew correctly on the MPC One: /dev/fb0 is 800x1280
//! XRGB8888, stride 3200, mounted with rotation 270 (left side up).
//!
//! This module does no I/O. It renders into a byte buffer the size of one
//! visible frame; `main.rs` decides where that buffer goes.

use crate::font;
use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::Color;
use std::convert::Infallible;

/// Each cell is 8 pixels wide and 16 tall, so an upper-half-block cell is two
/// square 8x8 pixels. 8x8 font glyphs are drawn stretched to 8x16.
pub const CELL_W: usize = 8;
pub const CELL_H: usize = 16;
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

    /// The MPC One's framebuffer, as its kernel reports it.
    pub fn mpc_one() -> Geometry {
        Geometry::new(800, 1280, 3200, 32).expect("valid")
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

/// A terminal colour as RGB. `Reset` is white text on black.
pub fn rgb(color: Color, foreground: bool) -> [u8; 3] {
    const ANSI: [[u8; 3]; 16] = [
        [0, 0, 0], [170, 0, 0], [0, 170, 0], [170, 85, 0], [0, 0, 170], [170, 0, 170], [0, 170, 170], [170, 170, 170],
        [85, 85, 85], [255, 85, 85], [85, 255, 85], [255, 255, 85], [85, 85, 255], [255, 85, 255], [85, 255, 255],
        [255, 255, 255],
    ];
    match color {
        Color::Reset => if foreground { [200, 200, 200] } else { [0, 0, 0] },
        Color::Black => ANSI[0],
        Color::Red => ANSI[1],
        Color::Green => ANSI[2],
        Color::Yellow => ANSI[3],
        Color::Blue => ANSI[4],
        Color::Magenta => ANSI[5],
        Color::Cyan => ANSI[6],
        Color::Gray => ANSI[7],
        Color::DarkGray => ANSI[8],
        Color::LightRed => ANSI[9],
        Color::LightGreen => ANSI[10],
        Color::LightYellow => ANSI[11],
        Color::LightBlue => ANSI[12],
        Color::LightMagenta => ANSI[13],
        Color::LightCyan => ANSI[14],
        Color::White => ANSI[15],
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Indexed(i) if i < 16 => ANSI[i as usize],
        Color::Indexed(i) if i >= 232 => {
            let v = 8 + (i - 232) * 10;
            [v, v, v]
        }
        Color::Indexed(i) => {
            let i = i - 16;
            let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            [level(i / 36), level((i / 6) % 6), level(i % 6)]
        }
    }
}

/// Which pixels of a cell are foreground: `rows(py)` gives a bit per column.
fn glyph_rows(symbol: &str) -> Option<[u8; 16]> {
    let mut chars = symbol.chars();
    let c = chars.next().unwrap_or(' ');
    if chars.next().is_some() {
        return None;
    }
    let mut rows = [0u8; 16];
    match c {
        '\u{2580}' => rows[..8].fill(0xff), // upper half block
        '\u{2584}' => rows[8..].fill(0xff), // lower half block
        '\u{2588}' => rows.fill(0xff),      // full block
        ' '..='~' => {
            let g = font::GLYPHS[(c as u8 - font::FIRST) as usize];
            for (i, r) in rows.iter_mut().enumerate() {
                *r = g[i / 2];
            }
        }
        _ => return None,
    }
    Some(rows)
}

/// A ratatui backend whose "terminal" is a framebuffer's pixels.
pub struct FramebufferBackend {
    geometry: Geometry,
    rotation: Rotation,
    cols: u16,
    rows: u16,
    frame: Vec<u8>,
    cursor: Position,
}

impl FramebufferBackend {
    pub fn new(geometry: Geometry, rotation: Rotation) -> Result<FramebufferBackend, String> {
        let (lw, lh) = geometry.logical_size(rotation);
        let (cols, rows) = (lw / CELL_W, lh / CELL_H);
        if cols < 8 || rows < 4 {
            return Err(format!("the screen ({lw}x{lh}) is too small"));
        }
        Ok(FramebufferBackend {
            geometry,
            rotation,
            cols: cols.min(u16::MAX as usize) as u16,
            rows: rows.min(u16::MAX as usize) as u16,
            frame: vec![0; geometry.frame_len()],
            cursor: Position::ORIGIN,
        })
    }

    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// One visible frame in the framebuffer's own layout, ready to write at offset 0.
    pub fn frame(&self) -> &[u8] {
        &self.frame
    }

    /// The colour the viewer sees at logical pixel (lx, ly).
    pub fn logical_pixel(&self, lx: usize, ly: usize) -> [u8; 3] {
        let (x, y) = self.geometry.native(self.rotation, lx, ly);
        let at = y * self.geometry.stride + x * self.geometry.bytes_per_pixel();
        match self.geometry.format {
            PixelFormat::Xrgb8888 => [self.frame[at + 2], self.frame[at + 1], self.frame[at]],
            PixelFormat::Rgb565 => {
                let v = u16::from_le_bytes([self.frame[at], self.frame[at + 1]]);
                [((v >> 11) << 3) as u8, (((v >> 5) & 0x3f) << 2) as u8, ((v & 0x1f) << 3) as u8]
            }
        }
    }

    fn put(&mut self, lx: usize, ly: usize, c: [u8; 3]) {
        let (x, y) = self.geometry.native(self.rotation, lx, ly);
        let at = y * self.geometry.stride + x * self.geometry.bytes_per_pixel();
        match self.geometry.format {
            PixelFormat::Xrgb8888 => self.frame[at..at + 4].copy_from_slice(&[c[2], c[1], c[0], 0]),
            PixelFormat::Rgb565 => {
                let v = ((c[0] as u16 >> 3) << 11) | ((c[1] as u16 >> 2) << 5) | (c[2] as u16 >> 3);
                self.frame[at..at + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
    }

    fn draw_cell(&mut self, col: u16, row: u16, cell: &Cell) {
        if col >= self.cols || row >= self.rows {
            return;
        }
        let mut fg = rgb(cell.fg, true);
        let mut bg = rgb(cell.bg, false);
        if cell.modifier.contains(ratatui::style::Modifier::REVERSED) {
            std::mem::swap(&mut fg, &mut bg);
        }
        // Anything the 8x8 font can't show is drawn as '?'.
        let rows = glyph_rows(cell.symbol()).or_else(|| glyph_rows("?")).unwrap();
        let (x0, y0) = (col as usize * CELL_W, row as usize * CELL_H);
        for (py, bits) in rows.iter().enumerate() {
            for px in 0..CELL_W {
                let on = bits & (1 << px) != 0;
                self.put(x0 + px, y0 + py, if on { fg } else { bg });
            }
        }
    }

    fn fill(&mut self, from_row: u16, to_row: u16) {
        let blank = Cell::default();
        for row in from_row..to_row.min(self.rows) {
            for col in 0..self.cols {
                self.draw_cell(col, row, &blank);
            }
        }
    }
}

impl Backend for FramebufferBackend {
    type Error = Infallible;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Infallible>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        for (x, y, cell) in content {
            self.draw_cell(x, y, cell);
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Infallible> {
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), Infallible> {
        Ok(())
    }

    fn get_cursor_position(&mut self) -> Result<Position, Infallible> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Infallible> {
        self.cursor = position.into();
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Infallible> {
        self.fill(0, self.rows);
        Ok(())
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Infallible> {
        match clear_type {
            ClearType::All => self.fill(0, self.rows),
            ClearType::AfterCursor => self.fill(self.cursor.y.saturating_add(1), self.rows),
            ClearType::BeforeCursor => self.fill(0, self.cursor.y),
            ClearType::CurrentLine | ClearType::UntilNewLine => self.fill(self.cursor.y, self.cursor.y.saturating_add(1)),
        }
        Ok(())
    }

    fn size(&self) -> Result<Size, Infallible> {
        Ok(Size::new(self.cols, self.rows))
    }

    fn window_size(&mut self) -> Result<WindowSize, Infallible> {
        let (lw, lh) = self.geometry.logical_size(self.rotation);
        Ok(WindowSize {
            columns_rows: Size::new(self.cols, self.rows),
            pixels: Size::new(lw.min(u16::MAX as usize) as u16, lh.min(u16::MAX as usize) as u16),
        })
    }

    /// Nothing to do: the frame is in memory. The caller writes `frame()` out.
    fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::style::Style;

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
        let g = Geometry::mpc_one();
        let r = Rotation::Deg270;
        assert_eq!(g.logical_size(r), (1280, 800));
        assert_eq!(g.native(r, 0, 0), (0, 1279), "top left");
        assert_eq!(g.native(r, 1279, 0), (0, 0), "top right");
        assert_eq!(g.native(r, 0, 799), (799, 1279), "bottom left");
        assert_eq!(g.native(r, 1279, 799), (799, 0), "bottom right");
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
    fn mpc_one_is_160_by_50_cells() {
        let mut b = FramebufferBackend::new(Geometry::mpc_one(), Rotation::Deg270).unwrap();
        assert_eq!(b.size().unwrap(), Size::new(160, 50));
        assert_eq!(b.window_size().unwrap().pixels, Size::new(1280, 800));
    }

    #[test]
    fn half_blocks_and_text_land_where_the_viewer_sees_them() {
        let b = FramebufferBackend::new(Geometry::mpc_one(), Rotation::Deg270).unwrap();
        let mut term = Terminal::new(b).unwrap();
        term.draw(|f| {
            let buf = f.buffer_mut();
            buf[(0, 0)].set_symbol("\u{2580}").set_fg(Color::Rgb(255, 0, 0)).set_bg(Color::Rgb(0, 0, 255));
            buf.set_string(1, 0, "A", Style::new().fg(Color::Rgb(0, 255, 0)).bg(Color::Black));
            buf[(159, 49)].set_symbol("\u{2588}").set_fg(Color::Rgb(9, 8, 7));
        })
        .unwrap();
        let b = term.backend();
        // Cell (0, 0): red on top, blue below.
        assert_eq!(b.logical_pixel(3, 2), [255, 0, 0]);
        assert_eq!(b.logical_pixel(3, 12), [0, 0, 255]);
        // "A" in cell (1, 0): font row 0 is 0x0c, so font pixels 2 and 3 are set; rows are doubled.
        assert_eq!(b.logical_pixel(8 + 2, 0), [0, 255, 0]);
        assert_eq!(b.logical_pixel(8 + 3, 1), [0, 255, 0]);
        assert_eq!(b.logical_pixel(8, 0), [0, 0, 0]);
        // The bottom right cell, as stored: B, G, R, X at native (799 - 15 .. 799, 0 .. 7).
        assert_eq!(b.logical_pixel(1279, 799), [9, 8, 7]);
        let (x, y) = b.geometry().native(Rotation::Deg270, 1279, 799);
        assert_eq!((x, y), (799, 0));
        assert_eq!(&b.frame()[x * 4..x * 4 + 4], &[7, 8, 9, 0]);
    }

    #[test]
    fn rgb565_round_trip() {
        let g = Geometry::new(800, 1280, 1600, 16).unwrap();
        let b = FramebufferBackend::new(g, Rotation::Deg270).unwrap();
        let mut term = Terminal::new(b).unwrap();
        term.draw(|f| {
            f.buffer_mut()[(0, 0)].set_symbol("\u{2588}").set_fg(Color::Rgb(248, 252, 248));
        })
        .unwrap();
        assert_eq!(term.backend().logical_pixel(0, 0), [248, 252, 248]);
    }

    #[test]
    fn colours() {
        assert_eq!(rgb(Color::Reset, false), [0, 0, 0]);
        assert_eq!(rgb(Color::Indexed(196), true), [255, 0, 0]);
        assert_eq!(rgb(Color::Indexed(232), true), [8, 8, 8]);
    }
}
