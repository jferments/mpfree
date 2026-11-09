//! M8 offscreen test card and display/touch coordinate model.
//!
//! NO DRM, fbdev, evdev, I2C, firmware, systemd or MPC device access.
//! The physical native panel is 800x1280; logical MPFree GUI is 1280x800.
//! Orientation and raw input axis ranges are *not yet calibrated*.
//! This module is a pure CPU image/geometry rehearsal on the host.
#![forbid(unsafe_code)]

pub const LOGICAL_WIDTH: usize = 1280;
pub const LOGICAL_HEIGHT: usize = 800;
pub const PANEL_WIDTH: usize = 800;
pub const PANEL_HEIGHT: usize = 1280;
const MAX_RGB_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayError {
    InvalidDimensions,
    AllocationLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarterTurn {
    Clockwise,
    CounterClockwise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: usize,
    pub y: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisRange {
    pub min: i32,
    pub max: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbCanvas {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl RgbCanvas {
    pub fn new(width: usize, height: usize) -> Result<Self, DisplayError> {
        if width == 0 || height == 0 {
            return Err(DisplayError::InvalidDimensions);
        }
        let bytes = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or(DisplayError::AllocationLimit)?;
        if bytes > MAX_RGB_BYTES {
            return Err(DisplayError::AllocationLimit);
        }
        Ok(Self { width, height, data: vec![0; bytes] })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn rgb(&self) -> &[u8] {
        &self.data
    }

    pub fn pixel(&self, p: Point) -> Option<[u8; 3]> {
        if p.x >= self.width || p.y >= self.height {
            return None;
        }
        let index = (p.y * self.width + p.x) * 3;
        Some([self.data[index], self.data[index + 1], self.data[index + 2]])
    }

    fn set(&mut self, x: usize, y: usize, color: [u8; 3]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let index = (y * self.width + x) * 3;
        self.data[index..index + 3].copy_from_slice(&color);
    }

    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: [u8; 3]) {
        let end_x = x.saturating_add(w).min(self.width);
        let end_y = y.saturating_add(h).min(self.height);
        for row in y.min(self.height)..end_y {
            for col in x.min(self.width)..end_x {
                self.set(col, row, color);
            }
        }
    }

    pub fn checksum_fnv1a64(&self) -> u64 {
        self.data.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
        })
    }

    /// Rotate an offscreen logical 1280x800 frame into a native 800x1280 image.
    /// No claim is made about the factory screen's physical orientation.
    pub fn rotate_for_panel(&self, turn: QuarterTurn) -> Result<Self, DisplayError> {
        if self.width != LOGICAL_WIDTH || self.height != LOGICAL_HEIGHT {
            return Err(DisplayError::InvalidDimensions);
        }
        let mut out = Self::new(PANEL_WIDTH, PANEL_HEIGHT)?;
        for y in 0..self.height {
            for x in 0..self.width {
                let (nx, ny) = match turn {
                    QuarterTurn::Clockwise => (self.height - 1 - y, x),
                    QuarterTurn::CounterClockwise => (y, self.width - 1 - x),
                };
                let color = self.pixel(Point { x, y }).expect("in-range pixel");
                out.set(nx, ny, color);
            }
        }
        Ok(out)
    }
}

fn glyph(letter: char) -> Option<[u8; 7]> {
    // Only the needed letters are encoded. Each row contains five pixels.
    let rows = match letter {
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        ' ' => [0; 7],
        _ => return None,
    };
    Some(rows)
}

fn draw_label(canvas: &mut RgbCanvas, x: usize, y: usize, label: &str) {
    const SCALE: usize = 4;
    for (glyph_index, letter) in label.chars().enumerate() {
        if let Some(rows) = glyph(letter) {
            let x0 = x + glyph_index * 6 * SCALE;
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (1 << (4 - col)) != 0 {
                        canvas.fill_rect(
                            x0 + col * SCALE, y + row * SCALE,
                            SCALE, SCALE, [238, 246, 255],
                        );
                    }
                }
            }
        }
    }
}

/// Known-color offscreen geometry test. This doesn't control a display.
pub fn render_test_card() -> Result<RgbCanvas, DisplayError> {
    let mut canvas = RgbCanvas::new(LOGICAL_WIDTH, LOGICAL_HEIGHT)?;
    canvas.fill_rect(0, 0, LOGICAL_WIDTH, LOGICAL_HEIGHT, [11, 20, 36]);
    for x in (0..LOGICAL_WIDTH).step_by(100) {
        canvas.fill_rect(x, 0, 1, LOGICAL_HEIGHT, [38, 58, 78]);
    }
    for y in (0..LOGICAL_HEIGHT).step_by(100) {
        canvas.fill_rect(0, y, LOGICAL_WIDTH, 1, [38, 58, 78]);
    }
    canvas.fill_rect(0, 0, LOGICAL_WIDTH, 72, [24, 43, 68]);
    canvas.fill_rect(0, LOGICAL_HEIGHT / 2, LOGICAL_WIDTH, 2, [88, 115, 152]);
    canvas.fill_rect(LOGICAL_WIDTH / 2, 0, 2, LOGICAL_HEIGHT, [88, 115, 152]);
    draw_label(&mut canvas, 438, 24, "MPFREE TEST");

    // Unmistakably different corner colors detect mirrored/rotated output.
    const MARK: usize = 72;
    canvas.fill_rect(0, 0, MARK, MARK, [255, 72, 72]); // top-left
    canvas.fill_rect(LOGICAL_WIDTH - MARK, 0, MARK, MARK, [72, 255, 88]); // top-right
    canvas.fill_rect(0, LOGICAL_HEIGHT - MARK, MARK, MARK, [64, 124, 255]); // bottom-left
    canvas.fill_rect(
        LOGICAL_WIDTH - MARK, LOGICAL_HEIGHT - MARK,
        MARK, MARK, [255, 210, 64], // bottom-right
    );
    Ok(canvas)
}

/// Map *configured* raw touch ranges into the logical GUI orientation.
/// No guessed ILI2117 2048-range constants: ranges must come from EVIOCGABS
/// on the real device, in a separately approved later diagnostic.
pub fn map_raw_touch(
    raw_x: i32,
    raw_y: i32,
    range_x: AxisRange,
    range_y: AxisRange,
    turn: QuarterTurn,
) -> Option<Point> {
    fn scale(raw: i32, range: AxisRange, pixels: usize) -> Option<usize> {
        if range.max <= range.min || raw < range.min || raw > range.max {
            return None;
        }
        let numerator = i64::from(raw - range.min) * (pixels as i64 - 1);
        let denominator = i64::from(range.max) - i64::from(range.min);
        Some((numerator / denominator) as usize)
    }
    let nx = scale(raw_x, range_x, PANEL_WIDTH)?;
    let ny = scale(raw_y, range_y, PANEL_HEIGHT)?;
    let (x, y) = match turn {
        QuarterTurn::Clockwise => (ny, PANEL_WIDTH - 1 - nx),
        QuarterTurn::CounterClockwise => (PANEL_HEIGHT - 1 - ny, nx),
    };
    Some(Point { x, y })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_limit_and_clipping() {
        assert_eq!(RgbCanvas::new(0, 10), Err(DisplayError::InvalidDimensions));
        assert_eq!(RgbCanvas::new(usize::MAX, 10), Err(DisplayError::AllocationLimit));
        let mut c = RgbCanvas::new(2, 3).unwrap();
        c.fill_rect(1, 2, usize::MAX, usize::MAX, [1, 2, 3]);
        assert_eq!(c.pixel(Point { x: 1, y: 2 }), Some([1, 2, 3]));
        assert_eq!(c.pixel(Point { x: 0, y: 2 }), Some([0, 0, 0]));
        assert_eq!(c.pixel(Point { x: 2, y: 0 }), None);
    }

    #[test]
    fn corner_colors_and_deterministic_hash() {
        let a = render_test_card().unwrap();
        let b = render_test_card().unwrap();
        assert_eq!(a.checksum_fnv1a64(), b.checksum_fnv1a64());
        assert_eq!(a.rgb().len(), 1280 * 800 * 3);
        assert_eq!(a.pixel(Point { x: 0, y: 0 }), Some([255, 72, 72]));
        assert_eq!(a.pixel(Point { x: 1279, y: 0 }), Some([72, 255, 88]));
        assert_eq!(a.pixel(Point { x: 0, y: 799 }), Some([64, 124, 255]));
        assert_eq!(a.pixel(Point { x: 1279, y: 799 }), Some([255, 210, 64]));
    }

    #[test]
    fn rotation_is_bijective_and_has_unambiguous_corners() {
        let a = render_test_card().unwrap();
        for turn in [QuarterTurn::Clockwise, QuarterTurn::CounterClockwise] {
            let b = a.rotate_for_panel(turn).unwrap();
            assert_eq!((b.width(), b.height()), (800, 1280));
            let mut rotated_xor = 0_u8;
            for pixel in b.rgb() {
                rotated_xor ^= *pixel;
            }
            let mut original_xor = 0_u8;
            for pixel in a.rgb() {
                original_xor ^= *pixel;
            }
            assert_eq!(rotated_xor, original_xor);
        }
        let cw = a.rotate_for_panel(QuarterTurn::Clockwise).unwrap();
        assert_eq!(cw.pixel(Point { x: 799, y: 0 }), a.pixel(Point { x: 0, y: 0 }));
        assert_eq!(cw.pixel(Point { x: 0, y: 1279 }), a.pixel(Point { x: 1279, y: 799 }));
        let ccw = a.rotate_for_panel(QuarterTurn::CounterClockwise).unwrap();
        assert_eq!(ccw.pixel(Point { x: 0, y: 1279 }), a.pixel(Point { x: 0, y: 0 }));
        assert_eq!(ccw.pixel(Point { x: 799, y: 0 }), a.pixel(Point { x: 1279, y: 799 }));
    }

    #[test]
    fn touch_mapping_requires_actual_ranges_and_applies_inverse_rotation() {
        let xr = AxisRange { min: 100, max: 899 }; // 800 range, *test fixture*
        let yr = AxisRange { min: 500, max: 1779 }; // 1280 range, *test fixture*
        assert_eq!(
            map_raw_touch(100, 500, xr, yr, QuarterTurn::Clockwise),
            Some(Point { x: 0, y: 799 })
        );
        assert_eq!(
            map_raw_touch(899, 1779, xr, yr, QuarterTurn::Clockwise),
            Some(Point { x: 1279, y: 0 })
        );
        assert_eq!(
            map_raw_touch(100, 500, xr, yr, QuarterTurn::CounterClockwise),
            Some(Point { x: 1279, y: 0 })
        );
        assert_eq!(map_raw_touch(99, 500, xr, yr, QuarterTurn::Clockwise), None);
        assert_eq!(
            map_raw_touch(100, 500, AxisRange { min: 5, max: 5 }, yr,
                          QuarterTurn::Clockwise),
            None
        );
    }
}
