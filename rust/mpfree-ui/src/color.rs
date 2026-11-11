//! Colours: 8-bit sRGB with alpha. Blending is done in sRGB, which is what
//! simple displays and most UI toolkits do.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// An opaque colour from 0xRRGGBB.
    pub const fn rgb(hex: u32) -> Color {
        Color {
            r: (hex >> 16) as u8,
            g: (hex >> 8) as u8,
            b: hex as u8,
            a: 255,
        }
    }

    pub const fn with_alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }

    /// A pad colour as the controller takes it (each channel 0-63, the range
    /// Akai's app uses for a resting pad), shown on screen at full range.
    pub const fn from_pad(rgb: [u8; 3]) -> Color {
        const fn up(v: u8) -> u8 {
            let v = if v > 63 { 63 } else { v };
            ((v as u32 * 255 + 31) / 63) as u8
        }
        Color {
            r: up(rgb[0]),
            g: up(rgb[1]),
            b: up(rgb[2]),
            a: 255,
        }
    }

    /// `t` = 0 gives self, 1 gives `other` (alpha mixes too).
    pub fn mix(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color {
            r: m(self.r, other.r),
            g: m(self.g, other.g),
            b: m(self.b, other.b),
            a: m(self.a, other.a),
        }
    }

    /// 0x00RRGGBB, the framebuffer's XRGB8888 word (alpha dropped).
    pub const fn xrgb(self) -> u32 {
        (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    pub const fn from_xrgb(p: u32) -> Color {
        Color::rgb(p & 0xFF_FFFF)
    }
}

/// Blend `c` over the XRGB pixel `dst` with extra coverage `cov` (0-255).
#[inline]
pub fn blend(dst: u32, c: Color, cov: u32) -> u32 {
    let a = c.a as u32 * cov;
    if a >= 255 * 255 {
        return c.xrgb();
    }
    if a == 0 {
        return dst;
    }
    // a is 0..65025; scale to 0..256 for shifts.
    let a = (a + 127) / 255 + 1; // 1..=256
    let inv = 256 - a;
    let ch = |shift: u32, src: u8| ((((dst >> shift) & 0xFF) * inv + src as u32 * a) >> 8) << shift;
    ch(16, c.r) | ch(8, c.g) | ch(0, c.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_colours_scale_to_full_range() {
        assert_eq!(Color::from_pad([63, 0, 32]), Color::rgb(0xFF0082));
        assert_eq!(Color::from_pad([200, 0, 0]).r, 255);
    }

    #[test]
    fn blending() {
        let red = Color::rgb(0xFF0000);
        assert_eq!(blend(0x000000, red, 255), 0xFF0000);
        assert_eq!(blend(0x123456, red, 0), 0x123456);
        let half = blend(0x000000, red, 128);
        assert!(((half >> 16) as i32 - 128).abs() <= 1, "{half:06X}");
        assert_eq!(blend(0x00FF00, red.with_alpha(0), 255), 0x00FF00);
        assert_eq!(
            Color::rgb(0x000000).mix(Color::rgb(0xFFFFFF), 0.5),
            Color::rgb(0x808080)
        );
    }
}
