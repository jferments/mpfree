//! A ratatui backend that draws character cells straight into a Linux
//! framebuffer's pixel layout, for screens with no text console (both of
//! Akai's MPC One kernels have `CONFIG_FRAMEBUFFER_CONSOLE` off).
//!
//! The geometry, rotation and 8x8 font come from `mpfree-screen-log` (kexec
//! test boot), which drew correctly on the MPC One: /dev/fb0 is 800x1280
//! XRGB8888, stride 3200, mounted with rotation 270 (left side up).
//!
//! This module does no I/O. It renders into a byte buffer the size of one
//! visible frame; `main.rs` decides where that buffer goes.

use mpfree_screen_log::font;
use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::Color;
use std::convert::Infallible;

/// Each cell is 4 pixels wide and 8 tall, so an upper-half-block cell is two
/// square 4x4 pixels: the MPC One's 1280x800 becomes 320x100 cells, a
/// 320x200-pixel canvas for the splash. Plain text (8x8 font glyphs squeezed
/// into 4x8) is only legible enough for a fallback; the splash draws no text cells.
pub const CELL_W: usize = 4;
pub const CELL_H: usize = 8;

pub use mpfree_screen_log::{Geometry, PixelFormat, Rotation};

/// The MPC One's framebuffer, as its kernel reports it: 800x1280, 32 bits, stride 3200.
pub fn mpc_one() -> Geometry {
    Geometry::new(800, 1280, 3200, 32).expect("valid")
}

/// A terminal colour as RGB. `Reset` is white text on black.
pub fn rgb(color: Color, foreground: bool) -> [u8; 3] {
    const ANSI: [[u8; 3]; 16] = [
        [0, 0, 0],
        [170, 0, 0],
        [0, 170, 0],
        [170, 85, 0],
        [0, 0, 170],
        [170, 0, 170],
        [0, 170, 170],
        [170, 170, 170],
        [85, 85, 85],
        [255, 85, 85],
        [85, 255, 85],
        [255, 255, 85],
        [85, 85, 255],
        [255, 85, 255],
        [85, 255, 255],
        [255, 255, 255],
    ];
    match color {
        Color::Reset => {
            if foreground {
                [200, 200, 200]
            } else {
                [0, 0, 0]
            }
        }
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

/// Which pixels of a cell are foreground: one row of bits per pixel row, bit
/// `x` for pixel column `x`.
fn glyph_rows(symbol: &str) -> Option<[u8; CELL_H]> {
    let mut chars = symbol.chars();
    let c = chars.next().unwrap_or(' ');
    if chars.next().is_some() {
        return None;
    }
    let mut rows = [0u8; CELL_H];
    match c {
        '\u{2580}' => rows[..CELL_H / 2].fill(0xff), // upper half block
        '\u{2584}' => rows[CELL_H / 2..].fill(0xff), // lower half block
        '\u{2588}' => rows.fill(0xff),               // full block
        ' '..='~' => {
            let g = font::GLYPHS[(c as u8 - font::FIRST) as usize];
            // Sample the 8x8 glyph at the cell's size.
            for (y, r) in rows.iter_mut().enumerate() {
                let line = g[y * 8 / CELL_H];
                *r = (0..CELL_W)
                    .filter(|x| line & (1 << (x * 8 / CELL_W)) != 0)
                    .fold(0, |acc, x| acc | (1 << x));
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
                [
                    ((v >> 11) << 3) as u8,
                    (((v >> 5) & 0x3f) << 2) as u8,
                    ((v & 0x1f) << 3) as u8,
                ]
            }
        }
    }

    /// The pixel in the framebuffer's own format, as stored.
    fn encode(&self, c: [u8; 3]) -> [u8; 4] {
        match self.geometry.format {
            PixelFormat::Xrgb8888 => [c[2], c[1], c[0], 0],
            PixelFormat::Rgb565 => {
                let v = ((c[0] as u16 >> 3) << 11) | ((c[1] as u16 >> 2) << 5) | (c[2] as u16 >> 3);
                let [lo, hi] = v.to_le_bytes();
                [lo, hi, 0, 0]
            }
        }
    }

    /// Byte offsets: of logical pixel (lx, ly), and the steps for one logical
    /// pixel right and one down (signed, since rotation can run them backwards).
    fn steps(&self, lx: usize, ly: usize) -> (usize, isize, isize) {
        let (x, y) = self.geometry.native(self.rotation, lx, ly);
        let (bpp, stride) = (
            self.geometry.bytes_per_pixel() as isize,
            self.geometry.stride as isize,
        );
        let at = y * self.geometry.stride + x * self.geometry.bytes_per_pixel();
        let (right, down) = match self.rotation {
            Rotation::Deg0 => (bpp, stride),
            Rotation::Deg90 => (stride, -bpp),
            Rotation::Deg180 => (-bpp, -stride),
            Rotation::Deg270 => (-stride, bpp),
        };
        (at, right, down)
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
        let rows = glyph_rows(cell.symbol())
            .or_else(|| glyph_rows("?"))
            .unwrap();
        let (fg, bg) = (self.encode(fg), self.encode(bg));
        let n = self.geometry.bytes_per_pixel();
        let (origin, right, down) = self.steps(col as usize * CELL_W, row as usize * CELL_H);
        for (py, bits) in rows.iter().enumerate() {
            let mut at = origin as isize + py as isize * down;
            for px in 0..CELL_W {
                let c = if bits & (1 << px) != 0 { &fg } else { &bg };
                let i = at as usize;
                self.frame[i..i + n].copy_from_slice(&c[..n]);
                at += right;
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
            ClearType::CurrentLine | ClearType::UntilNewLine => {
                self.fill(self.cursor.y, self.cursor.y.saturating_add(1))
            }
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
            pixels: Size::new(
                lw.min(u16::MAX as usize) as u16,
                lh.min(u16::MAX as usize) as u16,
            ),
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
    fn every_rotation_draws_where_logical_pixel_reads() {
        // The fast drawing path steps through memory by rotation; check it
        // against the plain per-pixel mapping, for both depths.
        for (bpp, stride) in [(32, 160), (16, 80)] {
            let g = Geometry::new(40, 48, stride, bpp).unwrap();
            for degrees in [0, 90, 180, 270] {
                let r = Rotation::from_degrees(degrees).unwrap();
                let mut term = Terminal::new(
                    FramebufferBackend::new(g, r).unwrap_or_else(|e| panic!("{degrees}: {e}")),
                )
                .unwrap();
                let size = term.size().unwrap();
                term.draw(|f| {
                    let buf = f.buffer_mut();
                    for y in 0..size.height {
                        for x in 0..size.width {
                            let v = (x * 40 + y * 8) as u8;
                            buf[(x, y)]
                                .set_symbol("\u{2580}")
                                .set_fg(Color::Rgb(v, 248, 0))
                                .set_bg(Color::Rgb(0, v, 248));
                        }
                    }
                })
                .unwrap();
                let b = term.backend();
                let (lw, lh) = g.logical_size(r);
                for ly in 0..lh {
                    for lx in 0..lw {
                        let (col, row) = ((lx / CELL_W) as u16, (ly / CELL_H) as u16);
                        if col >= size.width || row >= size.height {
                            continue;
                        }
                        let v = (col * 40 + row * 8) as u8;
                        let want = if ly % CELL_H < CELL_H / 2 {
                            [v, 248, 0]
                        } else {
                            [0, v, 248]
                        };
                        let got = b.logical_pixel(lx, ly);
                        let close = got
                            .iter()
                            .zip(want)
                            .all(|(a, b)| (*a as i32 - b as i32).abs() <= 7);
                        assert!(
                            close,
                            "{bpp} bits, {degrees}: ({lx},{ly}) got {got:?} want {want:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn mpc_one_is_320_by_100_cells() {
        let mut b = FramebufferBackend::new(mpc_one(), Rotation::Deg270).unwrap();
        assert_eq!(b.size().unwrap(), Size::new(320, 100));
        assert_eq!(b.window_size().unwrap().pixels, Size::new(1280, 800));
    }

    #[test]
    fn half_blocks_and_text_land_where_the_viewer_sees_them() {
        let b = FramebufferBackend::new(mpc_one(), Rotation::Deg270).unwrap();
        let mut term = Terminal::new(b).unwrap();
        term.draw(|f| {
            let buf = f.buffer_mut();
            buf[(0, 0)]
                .set_symbol("\u{2580}")
                .set_fg(Color::Rgb(255, 0, 0))
                .set_bg(Color::Rgb(0, 0, 255));
            buf.set_string(
                1,
                0,
                "A",
                Style::new().fg(Color::Rgb(0, 255, 0)).bg(Color::Black),
            );
            buf[(319, 99)]
                .set_symbol("\u{2588}")
                .set_fg(Color::Rgb(9, 8, 7));
        })
        .unwrap();
        let b = term.backend();
        // Cell (0, 0): red on top, blue below.
        assert_eq!(b.logical_pixel(3, 2), [255, 0, 0]);
        assert_eq!(b.logical_pixel(3, 6), [0, 0, 255]);
        // "A" in cell (1, 0): font row 0 is 0x0c (font pixels 2 and 3); cell column 1 samples font pixel 2.
        assert_eq!(b.logical_pixel(CELL_W + 1, 0), [0, 255, 0]);
        assert_eq!(b.logical_pixel(CELL_W, 0), [0, 0, 0]);
        // The bottom right cell, as stored: B, G, R, X.
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
            f.buffer_mut()[(0, 0)]
                .set_symbol("\u{2588}")
                .set_fg(Color::Rgb(248, 252, 248));
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
