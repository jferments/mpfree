//! Copy a `SoftCanvas` to the Linux framebuffer (/dev/fb0), turned to match
//! how the panel is mounted, rewriting only the rectangles that changed.
//!
//! It opens the framebuffer write-only, never creates or truncates anything,
//! and writes only inside the visible frame. The panel's mounting comes from
//! the caller (the MPC One's device tree says 270 degrees).

use crate::canvas::SoftCanvas;
use crate::geom::Rect;
use mpfree_screen_log::{Geometry, PixelFormat, Rotation};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::Path;

fn read_trimmed(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map(|s| s.trim().to_string()).map_err(|e| format!("{}: {e}", path.display()))
}

/// The framebuffer's size, stride and depth from its sysfs folder (the first
/// line of `modes`, then `stride` and `bits_per_pixel`).
pub fn geometry_from_sysfs(dir: &Path) -> Result<Geometry, String> {
    let modes = read_trimmed(&dir.join("modes"))?;
    let first = modes.lines().next().unwrap_or("");
    let size = first.split(':').nth(1).unwrap_or("");
    let digits: String = size.chars().take_while(|c| c.is_ascii_digit() || *c == 'x').collect();
    let (w, h) = digits.split_once('x').ok_or_else(|| format!("cannot read a size from modes {first:?}"))?;
    let num = |s: &str, what: &str| s.parse::<usize>().map_err(|_| format!("bad {what} {s:?}"));
    let stride = num(&read_trimmed(&dir.join("stride"))?, "stride")?;
    let bpp = read_trimmed(&dir.join("bits_per_pixel"))?.parse::<u32>().map_err(|e| format!("bits_per_pixel: {e}"))?;
    Geometry::new(num(w, "width")?, num(h, "height")?, stride, bpp)
}

/// Where each logical pixel goes in the framebuffer.
pub struct FbLayout {
    pub geometry: Geometry,
    pub rotation: Rotation,
}

impl FbLayout {
    pub fn logical_size(&self) -> (i32, i32) {
        let (w, h) = self.geometry.logical_size(self.rotation);
        (w as i32, h as i32)
    }

    /// For each native row the rectangle `r` (logical) touches: the row, the
    /// first native column, and the frame bytes for that run of pixels.
    pub fn runs(&self, canvas: &SoftCanvas, r: Rect, mut emit: impl FnMut(usize, usize, &[u8])) {
        let (lw, lh) = self.logical_size();
        assert_eq!((canvas.width(), canvas.height()), (lw, lh), "canvas does not match the screen");
        let r = r.intersect(Rect::new(0, 0, lw, lh));
        if r.is_empty() {
            return;
        }
        let g = &self.geometry;
        // Native bounding box of the rectangle's corners.
        let a = g.native(self.rotation, r.x as usize, r.y as usize);
        let b = g.native(self.rotation, (r.right() - 1) as usize, (r.bottom() - 1) as usize);
        let (nx0, nx1) = (a.0.min(b.0), a.0.max(b.0) + 1);
        let (ny0, ny1) = (a.1.min(b.1), a.1.max(b.1) + 1);
        let bpp = g.bytes_per_pixel();
        let mut row = vec![0u8; (nx1 - nx0) * bpp];
        let lw = lw as usize;
        for ny in ny0..ny1 {
            for (k, nx) in (nx0..nx1).enumerate() {
                let (lx, ly) = logical(g, self.rotation, nx, ny);
                let p = canvas.px[ly * lw + lx];
                match g.format {
                    PixelFormat::Xrgb8888 => row[k * 4..k * 4 + 4].copy_from_slice(&p.to_le_bytes()),
                    PixelFormat::Rgb565 => {
                        let (r, gg, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
                        let v = ((r >> 3) << 11 | (gg >> 2) << 5 | (b >> 3)) as u16;
                        row[k * 2..k * 2 + 2].copy_from_slice(&v.to_le_bytes());
                    }
                }
            }
            emit(ny, nx0, &row);
        }
    }
}

/// The logical pixel shown at native (nx, ny): the inverse of `Geometry::native`.
fn logical(g: &Geometry, rot: Rotation, nx: usize, ny: usize) -> (usize, usize) {
    let (w, h) = (g.width, g.height);
    match rot {
        Rotation::Deg0 => (nx, ny),
        Rotation::Deg90 => (ny, w - 1 - nx),
        Rotation::Deg180 => (w - 1 - nx, h - 1 - ny),
        Rotation::Deg270 => (h - 1 - ny, nx),
    }
}

/// /dev/fb0, open for writing.
pub struct FbTarget {
    file: File,
    pub layout: FbLayout,
}

impl FbTarget {
    /// Open `fb` write-only and ask the kernel to show it (`pan` 0,0 in
    /// `sysfs`, a mode set; nothing is stored).
    pub fn open(fb: &Path, sysfs: &Path, rotation: Rotation) -> Result<FbTarget, String> {
        let geometry = geometry_from_sysfs(sysfs)?;
        let file = OpenOptions::new().write(true).open(fb).map_err(|e| format!("{}: {e}", fb.display()))?;
        let _ = OpenOptions::new().write(true).open(sysfs.join("pan")).and_then(|mut f| f.write_all(b"0,0\n"));
        Ok(FbTarget { file, layout: FbLayout { geometry, rotation } })
    }

    pub fn logical_size(&self) -> (i32, i32) {
        self.layout.logical_size()
    }

    /// Copy the given rectangles of `canvas` to the screen.
    pub fn present(&mut self, canvas: &SoftCanvas, damage: &[Rect]) -> Result<(), String> {
        let stride = self.layout.geometry.stride;
        let bpp = self.layout.geometry.bytes_per_pixel();
        let mut err = None;
        for r in damage {
            self.layout.runs(canvas, *r, |ny, nx, bytes| {
                if err.is_none() {
                    if let Err(e) = self.file.write_all_at(bytes, (ny * stride + nx * bpp) as u64) {
                        err = Some(format!("writing the screen: {e}"));
                    }
                }
            });
        }
        err.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::Canvas;

    /// Render a canvas into a whole native frame through `runs`.
    fn frame(layout: &FbLayout, canvas: &SoftCanvas, damage: Rect) -> Vec<u8> {
        let g = &layout.geometry;
        let mut out = vec![0u8; g.frame_len()];
        let bpp = g.bytes_per_pixel();
        layout.runs(canvas, damage, |ny, nx, bytes| {
            let at = ny * g.stride + nx * bpp;
            out[at..at + bytes.len()].copy_from_slice(bytes);
        });
        out
    }

    #[test]
    fn matches_the_screen_log_mapping_for_every_rotation() {
        for rot in [Rotation::Deg0, Rotation::Deg90, Rotation::Deg180, Rotation::Deg270] {
            let g = Geometry::new(8, 12, 8 * 4 + 16, 32).unwrap();
            let layout = FbLayout { geometry: g, rotation: rot };
            let (lw, lh) = layout.logical_size();
            let mut c = SoftCanvas::new(lw, lh);
            for y in 0..lh {
                for x in 0..lw {
                    c.fill_rect(Rect::new(x, y, 1, 1), Color::rgb((x as u32) << 8 | y as u32));
                }
            }
            let f = frame(&layout, &c, Rect::new(0, 0, lw, lh));
            for ly in 0..lh as usize {
                for lx in 0..lw as usize {
                    let (nx, ny) = g.native(rot, lx, ly);
                    let at = ny * g.stride + nx * 4;
                    assert_eq!(&f[at..at + 4], &[ly as u8, lx as u8, 0, 0], "{rot:?} ({lx},{ly})");
                }
            }
        }
    }

    #[test]
    fn writes_only_the_damaged_part() {
        let g = Geometry::new(800, 1280, 3200, 32).unwrap();
        let layout = FbLayout { geometry: g, rotation: Rotation::Deg270 };
        assert_eq!(layout.logical_size(), (1280, 800));
        let mut c = SoftCanvas::new(1280, 800);
        c.fill_rect(Rect::new(0, 0, 1280, 800), Color::rgb(0xFFFFFF));
        let mut rows = 0;
        let mut bytes = 0;
        layout.runs(&c, Rect::new(100, 50, 30, 20), |_, _, b| {
            rows += 1;
            bytes += b.len();
        });
        assert_eq!((rows, bytes), (30, 30 * 20 * 4));
        let f = frame(&layout, &c, Rect::new(100, 50, 30, 20));
        assert_eq!(f.iter().filter(|&&b| b == 0xFF).count(), 30 * 20 * 3);
    }

    #[test]
    fn rgb565_frames() {
        let g = Geometry::new(4, 2, 8, 16).unwrap();
        let layout = FbLayout { geometry: g, rotation: Rotation::Deg0 };
        let mut c = SoftCanvas::new(4, 2);
        c.fill_rect(Rect::new(0, 0, 4, 2), Color::rgb(0xFF0000));
        let f = frame(&layout, &c, Rect::new(0, 0, 4, 2));
        assert_eq!(&f[0..2], &0xF800u16.to_le_bytes());
    }

    #[test]
    fn reads_geometry_from_sysfs() {
        let dir = std::env::temp_dir().join(format!("mpfree-ui-fb-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("modes"), "U:800x1280p-0\n").unwrap();
        fs::write(dir.join("stride"), "3200\n").unwrap();
        fs::write(dir.join("bits_per_pixel"), "32\n").unwrap();
        let g = geometry_from_sysfs(&dir).unwrap();
        assert_eq!((g.width, g.height, g.stride), (800, 1280, 3200));
        fs::remove_dir_all(&dir).unwrap();
    }
}
