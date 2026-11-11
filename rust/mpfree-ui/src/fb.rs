//! Copy a `SoftCanvas` to the Linux framebuffer (/dev/fb0), turned to match
//! how the panel is mounted, rewriting only the native rows that changed.
//!
//! It opens the framebuffer write-only, never creates or truncates anything,
//! and writes only inside the visible frame. The panel's mounting comes from
//! the caller (the MPC One's device tree says 270 degrees).
//!
//! Every write covers two or more whole native rows. The kernel's framebuffer
//! emulation for DRM drivers with a shadow buffer (Rockchip's, in 6.18) works
//! out what to copy to the panel from each write's offset and length
//! (`drm_fb_helper_memory_range_to_clip`). A write of one row that ends
//! exactly at the row's end comes out zero pixels wide, so that row never
//! reaches the panel. Hello MPC round 4 showed it: one write per row left the
//! old picture showing around the panels.

use crate::canvas::{Image, SoftCanvas};
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

    /// The native rows [first, end) that the logical rectangle `r` touches.
    pub fn native_rows(&self, r: Rect) -> Option<(usize, usize)> {
        let (lw, lh) = self.logical_size();
        let r = r.intersect(Rect::new(0, 0, lw, lh));
        if r.is_empty() {
            return None;
        }
        let g = &self.geometry;
        let a = g.native(self.rotation, r.x as usize, r.y as usize);
        let b = g.native(self.rotation, (r.right() - 1) as usize, (r.bottom() - 1) as usize);
        Some((a.1.min(b.1), a.1.max(b.1) + 1))
    }

    /// The native row spans to rewrite for `damage`, in order: overlapping or
    /// touching spans merged, and each at least two rows long (see the top of
    /// this file), unless the screen has only one row.
    pub fn row_spans(&self, damage: &[Rect]) -> Vec<(usize, usize)> {
        let height = self.geometry.height;
        let mut spans: Vec<(usize, usize)> = damage.iter().filter_map(|r| self.native_rows(*r)).collect();
        spans.sort_unstable();
        let mut out: Vec<(usize, usize)> = Vec::new();
        for (mut a, mut b) in spans {
            if b - a < 2 && height >= 2 {
                if b < height {
                    b += 1;
                } else {
                    a -= 1;
                }
            }
            match out.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => out.push((a, b)),
            }
        }
        out
    }

    /// The bytes of whole native rows [first, end): `stride` bytes per row,
    /// the visible pixels from `canvas` and zeros in any padding after them.
    pub fn rows(&self, canvas: &SoftCanvas, first: usize, end: usize, out: &mut Vec<u8>) {
        let (lw, lh) = self.logical_size();
        assert_eq!((canvas.width(), canvas.height()), (lw, lh), "canvas does not match the screen");
        let g = &self.geometry;
        let bpp = g.bytes_per_pixel();
        let lw = lw as usize;
        out.clear();
        out.resize((end - first) * g.stride, 0);
        for (k, ny) in (first..end).enumerate() {
            let row = &mut out[k * g.stride..k * g.stride + g.width * bpp];
            for nx in 0..g.width {
                let (lx, ly) = logical(g, self.rotation, nx, ny);
                let p = canvas.px[ly * lw + lx];
                match g.format {
                    PixelFormat::Xrgb8888 => row[nx * 4..nx * 4 + 4].copy_from_slice(&p.to_le_bytes()),
                    PixelFormat::Rgb565 => {
                        let (r, gg, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
                        let v = ((r >> 3) << 11 | (gg >> 2) << 5 | (b >> 3)) as u16;
                        row[nx * 2..nx * 2 + 2].copy_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
    }
}

impl FbLayout {
    /// The picture held in `frame` (a whole native frame, as read from the
    /// framebuffer), turned the way the viewer sees it.
    pub fn picture(&self, frame: &[u8]) -> Result<Image, String> {
        let g = &self.geometry;
        if frame.len() < g.frame_len() {
            return Err(format!("the frame is {} bytes; the screen needs {}", frame.len(), g.frame_len()));
        }
        let (lw, lh) = self.logical_size();
        let mut img = Image::new(lw, lh);
        for ly in 0..lh as usize {
            for lx in 0..lw as usize {
                let (nx, ny) = g.native(self.rotation, lx, ly);
                let at = ny * g.stride + nx * g.bytes_per_pixel();
                img.px[ly * lw as usize + lx] = match g.format {
                    PixelFormat::Xrgb8888 => {
                        u32::from_le_bytes([frame[at], frame[at + 1], frame[at + 2], frame[at + 3]]) & 0x00FF_FFFF
                    }
                    PixelFormat::Rgb565 => {
                        let v = u16::from_le_bytes([frame[at], frame[at + 1]]) as u32;
                        let (r, gg, b) = ((v >> 11) & 0x1F, (v >> 5) & 0x3F, v & 0x1F);
                        ((r << 3 | r >> 2) << 16) | ((gg << 2 | gg >> 4) << 8) | (b << 3 | b >> 2)
                    }
                };
            }
        }
        Ok(img)
    }
}

/// What the screen shows now, read from the framebuffer (opened read-only).
/// The startup screen leaves its last frame there, so the app's loading page
/// can carry on from it.
pub fn capture(fb: &Path, layout: &FbLayout) -> Result<Image, String> {
    let file = OpenOptions::new().read(true).open(fb).map_err(|e| format!("{}: {e}", fb.display()))?;
    let mut frame = vec![0u8; layout.geometry.frame_len()];
    file.read_exact_at(&mut frame, 0).map_err(|e| format!("reading the screen: {e}"))?;
    layout.picture(&frame)
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
    buf: Vec<u8>,
}

impl FbTarget {
    /// Open `fb` write-only and ask the kernel to show it (`pan` 0,0 in
    /// `sysfs`, a mode set; nothing is stored).
    pub fn open(fb: &Path, sysfs: &Path, rotation: Rotation) -> Result<FbTarget, String> {
        let geometry = geometry_from_sysfs(sysfs)?;
        let file = OpenOptions::new().write(true).open(fb).map_err(|e| format!("{}: {e}", fb.display()))?;
        let _ = OpenOptions::new().write(true).open(sysfs.join("pan")).and_then(|mut f| f.write_all(b"0,0\n"));
        Ok(FbTarget { file, layout: FbLayout { geometry, rotation }, buf: Vec::new() })
    }

    pub fn logical_size(&self) -> (i32, i32) {
        self.layout.logical_size()
    }

    /// Copy the given rectangles of `canvas` to the screen, as whole native
    /// rows, two or more per write.
    pub fn present(&mut self, canvas: &SoftCanvas, damage: &[Rect]) -> Result<(), String> {
        let stride = self.layout.geometry.stride;
        for (first, end) in self.layout.row_spans(damage) {
            self.layout.rows(canvas, first, end, &mut self.buf);
            self.file.write_all_at(&self.buf, (first * stride) as u64).map_err(|e| format!("writing the screen: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::Canvas;

    /// Render a canvas into a whole native frame, writing what `present`
    /// would write for `damage`.
    fn frame(layout: &FbLayout, canvas: &SoftCanvas, damage: Rect) -> Vec<u8> {
        let g = &layout.geometry;
        let mut out = vec![0u8; g.frame_len()];
        let mut buf = Vec::new();
        for (first, end) in layout.row_spans(&[damage]) {
            layout.rows(canvas, first, end, &mut buf);
            out[first * g.stride..end * g.stride].copy_from_slice(&buf);
        }
        out
    }

    /// What the kernel's DRM framebuffer emulation (6.18,
    /// drm_fb_helper_memory_range_to_clip) marks as changed for one write:
    /// (x1, y1, x2, y2) in native pixels.
    fn kernel_clip(g: &Geometry, off: usize, len: usize) -> (usize, usize, usize, usize) {
        let line = g.stride;
        let end = off + len;
        let (mut x1, y1, mut x2, y2) = (0, (off / line).min(g.height), g.width, end.div_ceil(line).min(g.height));
        if y2 - y1 == 1 {
            let bits = g.bytes_per_pixel() * 8;
            x1 = (off % line) * 8 / bits;
            x2 = ((end % line) * 8).div_ceil(bits);
        }
        (x1, y1, x2.max(x1), y2)
    }

    #[test]
    fn a_frame_read_back_is_the_picture_that_was_written() {
        for rot in [Rotation::Deg0, Rotation::Deg90, Rotation::Deg180, Rotation::Deg270] {
            for bpp in [32, 16] {
                let g = Geometry::new(6, 10, 6 * bpp as usize / 8 + 8, bpp).unwrap();
                let layout = FbLayout { geometry: g, rotation: rot };
                let (w, h) = layout.logical_size();
                let mut c = SoftCanvas::new(w, h);
                for (i, p) in c.px.iter_mut().enumerate() {
                    // Values that survive RGB565 exactly: 5 or 6 bits, repeated into the low bits.
                    let (r, gg, b) = ((i * 7 % 32) as u32, (i * 13 % 64) as u32, (i * 3 % 32) as u32);
                    *p = ((r << 3 | r >> 2) << 16) | ((gg << 2 | gg >> 4) << 8) | (b << 3 | b >> 2);
                }
                let f = frame(&layout, &c, Rect::new(0, 0, w, h));
                let img = layout.picture(&f).unwrap();
                assert_eq!((img.w, img.h), (w, h));
                assert_eq!(img.px, c.px, "{rot:?} {bpp} bpp");
            }
        }
        let layout = FbLayout { geometry: Geometry::new(4, 4, 16, 32).unwrap(), rotation: Rotation::Deg0 };
        assert!(layout.picture(&[0u8; 10]).is_err());
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
    fn writes_whole_rows_of_the_damaged_part_only() {
        let g = Geometry::new(800, 1280, 3200, 32).unwrap();
        let layout = FbLayout { geometry: g, rotation: Rotation::Deg270 };
        assert_eq!(layout.logical_size(), (1280, 800));
        let mut c = SoftCanvas::new(1280, 800);
        c.fill_rect(Rect::new(0, 0, 1280, 800), Color::rgb(0xFFFFFF));
        // Logical columns 100..130 are native rows 1150..1180.
        assert_eq!(layout.row_spans(&[Rect::new(100, 50, 30, 20)]), vec![(1150, 1180)]);
        let f = frame(&layout, &c, Rect::new(100, 50, 30, 20));
        assert_eq!(f.iter().filter(|&&b| b == 0xFF).count(), 30 * 800 * 3);
    }

    #[test]
    fn spans_merge_and_are_never_one_row() {
        let g = Geometry::new(800, 1280, 3200, 32).unwrap();
        let layout = FbLayout { geometry: g, rotation: Rotation::Deg270 };
        // One logical column: one native row, made two.
        assert_eq!(layout.row_spans(&[Rect::new(10, 0, 1, 5)]), vec![(1269, 1271)]);
        // The last native row grows upwards.
        assert_eq!(layout.row_spans(&[Rect::new(0, 0, 1, 5)]), vec![(1278, 1280)]);
        // Overlapping and touching spans merge; separate ones stay apart.
        let spans = layout.row_spans(&[Rect::new(100, 0, 30, 5), Rect::new(120, 0, 30, 5), Rect::new(500, 0, 10, 5),
                                       Rect::new(2000, 0, 10, 5)]);
        assert_eq!(spans, vec![(770, 780), (1130, 1180)]);
    }

    #[test]
    fn the_kernel_sees_every_damaged_pixel() {
        // Whatever the damage, each write must mark all of its rows as changed
        // across the whole width, or the panel keeps the old picture there.
        let g = Geometry::new(800, 1280, 3200, 32).unwrap();
        let layout = FbLayout { geometry: g, rotation: Rotation::Deg270 };
        for r in [Rect::new(0, 0, 1280, 800), Rect::new(10, 0, 1, 800), Rect::new(0, 0, 1, 1), Rect::new(1279, 799, 1, 1),
                  Rect::new(300, 200, 170, 150)] {
            for (first, end) in layout.row_spans(&[r]) {
                let (x1, y1, x2, y2) = kernel_clip(&g, first * g.stride, (end - first) * g.stride);
                assert_eq!((x1, y1, x2, y2), (0, first, 800, end), "{r:?}");
            }
        }
        // The old way, one write per row, shows the problem.
        assert_eq!(kernel_clip(&g, 5 * 3200, 3200), (0, 5, 0, 6));
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
