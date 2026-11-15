//! Copy a `SoftCanvas` to the Linux framebuffer (/dev/fb0), turned to match
//! how the panel is mounted, rewriting only the native rows that changed.
//!
//! [`FbTarget`] keeps its own copy of what the panel shows, in the panel's
//! layout, and turns only the changed rectangles into it; the rows it
//! writes then come straight from that copy. Turned 90 or 270 degrees, a
//! changed rectangle's width is a count of whole native rows, so turning
//! whole rows from the canvas would turn a whole column of the screen for
//! every changed pixel column (a chip in the top bar: 120 rows of 800
//! pixels, to change 120 x 40).
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
use std::path::{Path, PathBuf};

fn read_trimmed(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The framebuffer's size, stride and depth from its sysfs folder (the first
/// line of `modes`, then `stride` and `bits_per_pixel`).
pub fn geometry_from_sysfs(dir: &Path) -> Result<Geometry, String> {
    let modes = read_trimmed(&dir.join("modes"))?;
    let first = modes.lines().next().unwrap_or("");
    let size = first.split(':').nth(1).unwrap_or("");
    let digits: String = size
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == 'x')
        .collect();
    let (w, h) = digits
        .split_once('x')
        .ok_or_else(|| format!("cannot read a size from modes {first:?}"))?;
    let num = |s: &str, what: &str| s.parse::<usize>().map_err(|_| format!("bad {what} {s:?}"));
    let stride = num(&read_trimmed(&dir.join("stride"))?, "stride")?;
    let bpp = read_trimmed(&dir.join("bits_per_pixel"))?
        .parse::<u32>()
        .map_err(|e| format!("bits_per_pixel: {e}"))?;
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

    /// The native columns [x0, x1) and rows [y0, y1) that the logical
    /// rectangle `r` covers, cut to the screen.
    pub fn native_rect(&self, r: Rect) -> Option<NativeRect> {
        let (lw, lh) = self.logical_size();
        let r = r.intersect(Rect::new(0, 0, lw, lh));
        if r.is_empty() {
            return None;
        }
        let g = &self.geometry;
        let a = g.native(self.rotation, r.x as usize, r.y as usize);
        let b = g.native(
            self.rotation,
            (r.right() - 1) as usize,
            (r.bottom() - 1) as usize,
        );
        Some(NativeRect {
            x0: a.0.min(b.0),
            x1: a.0.max(b.0) + 1,
            y0: a.1.min(b.1),
            y1: a.1.max(b.1) + 1,
        })
    }

    /// The native rows [first, end) that the logical rectangle `r` touches.
    pub fn native_rows(&self, r: Rect) -> Option<(usize, usize)> {
        self.native_rect(r).map(|n| (n.y0, n.y1))
    }

    /// The native row spans to rewrite for `damage`, in order: overlapping or
    /// touching spans merged, and each at least two rows long (see the top of
    /// this file), unless the screen has only one row.
    pub fn row_spans(&self, damage: &[Rect]) -> Vec<(usize, usize)> {
        let height = self.geometry.height;
        let mut spans: Vec<(usize, usize)> =
            damage.iter().filter_map(|r| self.native_rows(*r)).collect();
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
        let g = &self.geometry;
        out.clear();
        out.resize((end - first) * g.stride, 0);
        let all = NativeRect {
            x0: 0,
            x1: g.width,
            y0: first,
            y1: end,
        };
        self.turn_into(canvas, all, out, first);
    }

    /// Turns the logical rectangle `r` of `canvas` into `frame`, a whole
    /// native frame (every row, `stride` bytes each), leaving the rest of
    /// `frame` as it was.
    pub fn turn_rect(&self, canvas: &SoftCanvas, r: Rect, frame: &mut [u8]) {
        assert_eq!(
            frame.len(),
            self.geometry.frame_len(),
            "not a whole native frame"
        );
        if let Some(n) = self.native_rect(r) {
            self.turn_into(canvas, n, frame, 0);
        }
    }

    /// Turns the native rectangle `n` of the picture in `canvas` into `out`,
    /// whose first row is native row `out_row`.
    ///
    /// Turned 90 or 270 degrees, a native row is a column of the canvas, so
    /// it reads the canvas in bands of [`BAND`] native rows: for each native
    /// column, the band's pixels sit side by side in one canvas row, which
    /// the cache serves in one or two lines instead of one line per pixel.
    fn turn_into(&self, canvas: &SoftCanvas, n: NativeRect, out: &mut [u8], out_row: usize) {
        let (lw, lh) = self.logical_size();
        assert_eq!(
            (canvas.width(), canvas.height()),
            (lw, lh),
            "canvas does not match the screen"
        );
        let g = &self.geometry;
        let (bpp, stride, format) = (g.bytes_per_pixel(), g.stride, g.format);
        let lw = lw as usize;
        let px = &canvas.px;
        match self.rotation {
            Rotation::Deg0 | Rotation::Deg180 => {
                for ny in n.y0..n.y1 {
                    let base = (ny - out_row) * stride;
                    let (_, ly) = logical(g, self.rotation, 0, ny);
                    let row = &px[ly * lw..ly * lw + lw];
                    if self.rotation == Rotation::Deg0 {
                        for nx in n.x0..n.x1 {
                            put(out, base + nx * bpp, row[nx], format);
                        }
                    } else {
                        for nx in n.x0..n.x1 {
                            put(out, base + nx * bpp, row[g.width - 1 - nx], format);
                        }
                    }
                }
            }
            Rotation::Deg90 | Rotation::Deg270 => {
                let up = self.rotation == Rotation::Deg90;
                let mut ny0 = n.y0;
                while ny0 < n.y1 {
                    let band = BAND.min(n.y1 - ny0);
                    let base = (ny0 - out_row) * stride;
                    for nx in n.x0..n.x1 {
                        // Deg90: lx = ny, rising down the band; Deg270:
                        // lx = h - 1 - ny, falling.
                        let (lx0, ly) = logical(g, self.rotation, nx, ny0);
                        let src = if up {
                            &px[ly * lw + lx0..ly * lw + lx0 + band]
                        } else {
                            &px[ly * lw + lx0 + 1 - band..ly * lw + lx0 + 1]
                        };
                        for k in 0..band {
                            let p = if up { src[k] } else { src[band - 1 - k] };
                            put(out, base + k * stride + nx * bpp, p, format);
                        }
                    }
                    ny0 += band;
                }
            }
        }
    }
}

/// A rectangle of the panel in its own layout: columns [x0, x1), rows
/// [y0, y1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeRect {
    pub x0: usize,
    pub x1: usize,
    pub y0: usize,
    pub y1: usize,
}

/// Native rows read together when the picture is turned 90 or 270 degrees.
const BAND: usize = 16;

/// One XRGB8888 pixel at `off` in `out`, in the framebuffer's format.
#[inline]
fn put(out: &mut [u8], off: usize, p: u32, format: PixelFormat) {
    match format {
        PixelFormat::Xrgb8888 => out[off..off + 4].copy_from_slice(&p.to_le_bytes()),
        PixelFormat::Rgb565 => {
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            let v = ((r >> 3) << 11 | (g >> 2) << 5 | (b >> 3)) as u16;
            out[off..off + 2].copy_from_slice(&v.to_le_bytes());
        }
    }
}

impl FbLayout {
    /// The picture held in `frame` (a whole native frame, as read from the
    /// framebuffer), turned the way the viewer sees it.
    pub fn picture(&self, frame: &[u8]) -> Result<Image, String> {
        let g = &self.geometry;
        if frame.len() < g.frame_len() {
            return Err(format!(
                "the frame is {} bytes; the screen needs {}",
                frame.len(),
                g.frame_len()
            ));
        }
        let (lw, lh) = self.logical_size();
        let mut img = Image::new(lw, lh);
        for ly in 0..lh as usize {
            for lx in 0..lw as usize {
                let (nx, ny) = g.native(self.rotation, lx, ly);
                let at = ny * g.stride + nx * g.bytes_per_pixel();
                img.px[ly * lw as usize + lx] = match g.format {
                    PixelFormat::Xrgb8888 => {
                        u32::from_le_bytes([frame[at], frame[at + 1], frame[at + 2], frame[at + 3]])
                            & 0x00FF_FFFF
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
    let file = OpenOptions::new()
        .read(true)
        .open(fb)
        .map_err(|e| format!("{}: {e}", fb.display()))?;
    let mut frame = vec![0u8; layout.geometry.frame_len()];
    file.read_exact_at(&mut frame, 0)
        .map_err(|e| format!("reading the screen: {e}"))?;
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
    /// What the panel shows, in its own layout (a whole native frame), as
    /// this target last wrote it; empty until the first frame. Rows are
    /// written from here, so only the changed rectangles need turning.
    shown: Vec<u8>,
    pan: PathBuf,
}

impl FbTarget {
    /// Open `fb` write-only and ask the kernel to show it (`pan` 0,0 in
    /// `sysfs`, a mode set; nothing is stored).
    pub fn open(fb: &Path, sysfs: &Path, rotation: Rotation) -> Result<FbTarget, String> {
        let geometry = geometry_from_sysfs(sysfs)?;
        let file = OpenOptions::new()
            .write(true)
            .open(fb)
            .map_err(|e| format!("{}: {e}", fb.display()))?;
        let t = FbTarget {
            file,
            layout: FbLayout { geometry, rotation },
            shown: Vec::new(),
            pan: sysfs.join("pan"),
        };
        t.show();
        Ok(t)
    }

    /// Ask the kernel to show the framebuffer again (`pan` 0,0: a mode set
    /// in RAM, nothing stored), as after the GPU's process has had the
    /// screen. The kernel does nothing while another program holds the
    /// display (DRM master); best effort.
    pub fn show(&self) {
        let _ = OpenOptions::new()
            .write(true)
            .open(&self.pan)
            .and_then(|mut f| f.write_all(b"0,0\n"));
    }

    pub fn logical_size(&self) -> (i32, i32) {
        self.layout.logical_size()
    }

    /// Copy a whole native frame, already in the framebuffer's own layout
    /// (`frame_len` bytes: every row, `stride` bytes each), in one write.
    /// For a picture drawn elsewhere in the panel's orientation (the GPU's).
    pub fn present_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        let want = self.layout.geometry.frame_len();
        if frame.len() != want {
            return Err(format!(
                "a frame of {} bytes for a screen of {want}",
                frame.len()
            ));
        }
        // The next `present` turns the whole canvas again.
        self.shown.clear();
        self.file
            .write_all_at(frame, 0)
            .map_err(|e| format!("writing the screen: {e}"))
    }

    /// Copy the given rectangles of `canvas` to the screen, as whole native
    /// rows, two or more per write. Says how many native rows it wrote.
    ///
    /// Only the rectangles are turned, into the copy of what the panel
    /// shows; the first frame turns the whole canvas, so every row written
    /// is whole. Everything outside the rectangles must still be as the
    /// last frame left it, which painting through `Ui` keeps true.
    pub fn present(&mut self, canvas: &SoftCanvas, damage: &[Rect]) -> Result<usize, String> {
        let spans = self.layout.row_spans(damage);
        if spans.is_empty() {
            return Ok(0);
        }
        let g = &self.layout.geometry;
        let (stride, height) = (g.stride, g.height);
        if self.shown.len() != g.frame_len() {
            self.layout.rows(canvas, 0, height, &mut self.shown);
        } else {
            for r in damage {
                self.layout.turn_rect(canvas, *r, &mut self.shown);
            }
        }
        let mut rows = 0;
        for (first, end) in spans {
            self.file
                .write_all_at(
                    &self.shown[first * stride..end * stride],
                    (first * stride) as u64,
                )
                .map_err(|e| format!("writing the screen: {e}"))?;
            rows += end - first;
        }
        Ok(rows)
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
        let (mut x1, y1, mut x2, y2) = (
            0,
            (off / line).min(g.height),
            g.width,
            end.div_ceil(line).min(g.height),
        );
        if y2 - y1 == 1 {
            let bits = g.bytes_per_pixel() * 8;
            x1 = (off % line) * 8 / bits;
            x2 = ((end % line) * 8).div_ceil(bits);
        }
        (x1, y1, x2.max(x1), y2)
    }

    /// The bytes `rows` wrote before it read in bands: one pixel at a time.
    fn rows_one_by_one(
        layout: &FbLayout,
        canvas: &SoftCanvas,
        first: usize,
        end: usize,
    ) -> Vec<u8> {
        let g = &layout.geometry;
        let lw = canvas.width() as usize;
        let mut out = vec![0u8; (end - first) * g.stride];
        for (k, ny) in (first..end).enumerate() {
            for nx in 0..g.width {
                let (lx, ly) = logical(g, layout.rotation, nx, ny);
                put(
                    &mut out,
                    k * g.stride + nx * g.bytes_per_pixel(),
                    canvas.px[ly * lw + lx],
                    g.format,
                );
            }
        }
        out
    }

    fn noise_canvas(w: i32, h: i32) -> SoftCanvas {
        let mut c = SoftCanvas::new(w, h);
        let mut x = 0x2545F491u32;
        for p in c.px.iter_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *p = x & 0xFF_FFFF;
        }
        c
    }

    #[test]
    fn banded_rows_are_the_same_bytes_as_one_pixel_at_a_time() {
        for rot in [
            Rotation::Deg0,
            Rotation::Deg90,
            Rotation::Deg180,
            Rotation::Deg270,
        ] {
            for bpp in [32, 16] {
                // Sizes that are not a multiple of the band, with padding.
                let (nw, nh) = (37usize, 53usize);
                let g = Geometry::new(nw, nh, nw * bpp as usize / 8 + 12, bpp).unwrap();
                let layout = FbLayout {
                    geometry: g,
                    rotation: rot,
                };
                let (w, h) = layout.logical_size();
                let c = noise_canvas(w, h);
                for (first, end) in [(0, nh), (0, 1), (5, 6), (3, 40), (16, 33), (50, 53)] {
                    let mut out = Vec::new();
                    layout.rows(&c, first, end, &mut out);
                    assert!(
                        out == rows_one_by_one(&layout, &c, first, end),
                        "{rot:?} {bpp} bpp rows {first}..{end}"
                    );
                }
            }
        }
    }

    /// A tiny xorshift for test pictures and rectangles.
    struct Rng(u32);

    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0
        }

        fn below(&mut self, n: i32) -> i32 {
            (self.next() % n as u32) as i32
        }

        /// A rectangle that may hang off any edge, or be empty.
        fn rect(&mut self, w: i32, h: i32) -> Rect {
            Rect::new(
                self.below(w + 10) - 5,
                self.below(h + 10) - 5,
                self.below(w / 2 + 2),
                self.below(h / 2 + 2),
            )
        }
    }

    #[test]
    fn turning_only_the_changed_rectangles_keeps_the_whole_frame() {
        for rot in [
            Rotation::Deg0,
            Rotation::Deg90,
            Rotation::Deg180,
            Rotation::Deg270,
        ] {
            for bpp in [32, 16] {
                // Sizes that are not a multiple of the band, with padding.
                let (nw, nh) = (37usize, 53usize);
                let g = Geometry::new(nw, nh, nw * bpp as usize / 8 + 12, bpp).unwrap();
                let layout = FbLayout {
                    geometry: g,
                    rotation: rot,
                };
                let (w, h) = layout.logical_size();
                let mut c = noise_canvas(w, h);
                let mut shown = Vec::new();
                layout.rows(&c, 0, nh, &mut shown);
                let mut rng = Rng(0x9E37_79B9);
                let mut whole = Vec::new();
                for step in 0..60 {
                    let r = rng.rect(w, h);
                    let v = rng.next() & 0xFF_FFFF;
                    c.fill_rect(r, Color::from_xrgb(v));
                    layout.turn_rect(&c, r, &mut shown);
                    layout.rows(&c, 0, nh, &mut whole);
                    assert!(shown == whole, "{rot:?} {bpp} bpp, step {step}, {r:?}");
                }
            }
        }
    }

    /// A framebuffer file and its sysfs folder for `FbTarget`.
    fn fake_fb(name: &str, g: &Geometry) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("mpfree-ui-fb-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("modes"),
            format!("U:{}x{}p-0\n", g.width, g.height),
        )
        .unwrap();
        fs::write(dir.join("stride"), format!("{}\n", g.stride)).unwrap();
        fs::write(
            dir.join("bits_per_pixel"),
            format!("{}\n", g.bytes_per_pixel() * 8),
        )
        .unwrap();
        fs::write(dir.join("pan"), "").unwrap();
        let fb = dir.join("fb");
        fs::write(&fb, vec![0u8; g.frame_len()]).unwrap();
        (dir, fb)
    }

    #[test]
    fn the_screen_gets_the_same_bytes_as_when_whole_rows_were_turned() {
        // What `present` wrote before it kept a copy of the panel: each span
        // of whole rows turned from the canvas.
        let g = Geometry::new(37, 53, 37 * 4 + 12, 32).unwrap();
        let (dir, fb) = fake_fb("same", &g);
        let mut t = FbTarget::open(&fb, &dir, Rotation::Deg270).unwrap();
        let (w, h) = t.logical_size();
        let mut c = noise_canvas(w, h);
        let mut expected = vec![0u8; g.frame_len()];
        let mut rng = Rng(0x2545_F491);
        let mut buf = Vec::new();
        for step in 0..40 {
            let damage: Vec<Rect> = match step {
                // Nothing at first but a little: the first frame still
                // writes whole rows of the canvas.
                0 => vec![Rect::new(3, 4, 5, 6)],
                _ => (0..1 + rng.below(3)).map(|_| rng.rect(w, h)).collect(),
            };
            for r in &damage {
                let v = rng.next() & 0xFF_FFFF;
                c.fill_rect(*r, Color::from_xrgb(v));
            }
            let mut rows = 0;
            for (first, end) in t.layout.row_spans(&damage) {
                t.layout.rows(&c, first, end, &mut buf);
                expected[first * g.stride..end * g.stride].copy_from_slice(&buf);
                rows += end - first;
            }
            assert_eq!(t.present(&c, &damage).unwrap(), rows, "step {step}");
            assert!(
                fs::read(&fb).unwrap() == expected,
                "step {step}: {damage:?}"
            );
        }
        // A whole frame from elsewhere: the next frame turns the whole
        // canvas again, as before.
        t.present_frame(&vec![7u8; g.frame_len()]).unwrap();
        let r = Rect::new(10, 10, 4, 4);
        t.present(&c, &[r]).unwrap();
        let (first, end) = t.layout.native_rows(r).unwrap();
        t.layout.rows(&c, first, end, &mut buf);
        let now = fs::read(&fb).unwrap();
        assert!(now[first * g.stride..end * g.stride] == buf[..]);
        assert!(t.present(&c, &[]).unwrap() == 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The panel's own layout and the turn's cost for damage a frame
    /// commonly has, turning whole rows (before) and only the rectangles
    /// (now). `cargo test --release -p mpfree-ui turn_speed -- --ignored
    /// --nocapture`
    #[test]
    #[ignore]
    fn turn_speed() {
        let g = Geometry::new(800, 1280, 3200, 32).unwrap();
        let layout = FbLayout {
            geometry: g,
            rotation: Rotation::Deg270,
        };
        let (w, h) = layout.logical_size();
        let c = noise_canvas(w, h);
        let mut shown = Vec::new();
        layout.rows(&c, 0, g.height, &mut shown);
        let mut buf = Vec::new();
        let cases = [
            ("a top-bar chip 120x40", vec![Rect::new(600, 0, 120, 40)]),
            ("the top bar 1280x48", vec![Rect::new(0, 0, 1280, 48)]),
            ("a meter 24x300", vec![Rect::new(1200, 300, 24, 300)]),
            ("a playhead 3x400", vec![Rect::new(640, 200, 3, 400)]),
            ("a panel 600x400", vec![Rect::new(340, 200, 600, 400)]),
            ("the whole screen", vec![Rect::new(0, 0, w, h)]),
        ];
        let n = 50;
        for (what, damage) in cases {
            let t = std::time::Instant::now();
            for _ in 0..n {
                for (first, end) in layout.row_spans(&damage) {
                    layout.rows(&c, first, end, &mut buf);
                    std::hint::black_box(&buf);
                }
            }
            let before = t.elapsed() / n;
            let t = std::time::Instant::now();
            for _ in 0..n {
                for r in &damage {
                    layout.turn_rect(&c, *r, &mut shown);
                }
                std::hint::black_box(&shown);
            }
            let now = t.elapsed() / n;
            eprintln!("{what}: whole rows {before:?}, the rectangle only {now:?}");
        }
    }

    /// `cargo test --release -p mpfree-ui rows_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn rows_speed() {
        let g = Geometry::new(800, 1280, 3200, 32).unwrap();
        let layout = FbLayout {
            geometry: g,
            rotation: Rotation::Deg270,
        };
        let (w, h) = layout.logical_size();
        let c = noise_canvas(w, h);
        let mut out = Vec::new();
        let n = 30;
        let t = std::time::Instant::now();
        for _ in 0..n {
            layout.rows(&c, 0, 1280, &mut out);
        }
        let banded = t.elapsed() / n;
        let t = std::time::Instant::now();
        for _ in 0..n {
            std::hint::black_box(rows_one_by_one(&layout, &c, 0, 1280));
        }
        let single = t.elapsed() / n;
        eprintln!(
            "a whole 800x1280 frame turned 270: banded {banded:?}, one pixel at a time {single:?}"
        );
    }

    #[test]
    fn a_frame_read_back_is_the_picture_that_was_written() {
        for rot in [
            Rotation::Deg0,
            Rotation::Deg90,
            Rotation::Deg180,
            Rotation::Deg270,
        ] {
            for bpp in [32, 16] {
                let g = Geometry::new(6, 10, 6 * bpp as usize / 8 + 8, bpp).unwrap();
                let layout = FbLayout {
                    geometry: g,
                    rotation: rot,
                };
                let (w, h) = layout.logical_size();
                let mut c = SoftCanvas::new(w, h);
                for (i, p) in c.px.iter_mut().enumerate() {
                    // Values that survive RGB565 exactly: 5 or 6 bits, repeated into the low bits.
                    let (r, gg, b) = (
                        (i * 7 % 32) as u32,
                        (i * 13 % 64) as u32,
                        (i * 3 % 32) as u32,
                    );
                    *p = ((r << 3 | r >> 2) << 16) | ((gg << 2 | gg >> 4) << 8) | (b << 3 | b >> 2);
                }
                let f = frame(&layout, &c, Rect::new(0, 0, w, h));
                let img = layout.picture(&f).unwrap();
                assert_eq!((img.w, img.h), (w, h));
                assert_eq!(img.px, c.px, "{rot:?} {bpp} bpp");
            }
        }
        let layout = FbLayout {
            geometry: Geometry::new(4, 4, 16, 32).unwrap(),
            rotation: Rotation::Deg0,
        };
        assert!(layout.picture(&[0u8; 10]).is_err());
    }

    #[test]
    fn matches_the_screen_log_mapping_for_every_rotation() {
        for rot in [
            Rotation::Deg0,
            Rotation::Deg90,
            Rotation::Deg180,
            Rotation::Deg270,
        ] {
            let g = Geometry::new(8, 12, 8 * 4 + 16, 32).unwrap();
            let layout = FbLayout {
                geometry: g,
                rotation: rot,
            };
            let (lw, lh) = layout.logical_size();
            let mut c = SoftCanvas::new(lw, lh);
            for y in 0..lh {
                for x in 0..lw {
                    c.fill_rect(
                        Rect::new(x, y, 1, 1),
                        Color::rgb((x as u32) << 8 | y as u32),
                    );
                }
            }
            let f = frame(&layout, &c, Rect::new(0, 0, lw, lh));
            for ly in 0..lh as usize {
                for lx in 0..lw as usize {
                    let (nx, ny) = g.native(rot, lx, ly);
                    let at = ny * g.stride + nx * 4;
                    assert_eq!(
                        &f[at..at + 4],
                        &[ly as u8, lx as u8, 0, 0],
                        "{rot:?} ({lx},{ly})"
                    );
                }
            }
        }
    }

    #[test]
    fn writes_whole_rows_of_the_damaged_part_only() {
        let g = Geometry::new(800, 1280, 3200, 32).unwrap();
        let layout = FbLayout {
            geometry: g,
            rotation: Rotation::Deg270,
        };
        assert_eq!(layout.logical_size(), (1280, 800));
        let mut c = SoftCanvas::new(1280, 800);
        c.fill_rect(Rect::new(0, 0, 1280, 800), Color::rgb(0xFFFFFF));
        // Logical columns 100..130 are native rows 1150..1180.
        assert_eq!(
            layout.row_spans(&[Rect::new(100, 50, 30, 20)]),
            vec![(1150, 1180)]
        );
        let f = frame(&layout, &c, Rect::new(100, 50, 30, 20));
        assert_eq!(f.iter().filter(|&&b| b == 0xFF).count(), 30 * 800 * 3);
    }

    #[test]
    fn spans_merge_and_are_never_one_row() {
        let g = Geometry::new(800, 1280, 3200, 32).unwrap();
        let layout = FbLayout {
            geometry: g,
            rotation: Rotation::Deg270,
        };
        // One logical column: one native row, made two.
        assert_eq!(
            layout.row_spans(&[Rect::new(10, 0, 1, 5)]),
            vec![(1269, 1271)]
        );
        // The last native row grows upwards.
        assert_eq!(
            layout.row_spans(&[Rect::new(0, 0, 1, 5)]),
            vec![(1278, 1280)]
        );
        // Overlapping and touching spans merge; separate ones stay apart.
        let spans = layout.row_spans(&[
            Rect::new(100, 0, 30, 5),
            Rect::new(120, 0, 30, 5),
            Rect::new(500, 0, 10, 5),
            Rect::new(2000, 0, 10, 5),
        ]);
        assert_eq!(spans, vec![(770, 780), (1130, 1180)]);
    }

    #[test]
    fn the_kernel_sees_every_damaged_pixel() {
        // Whatever the damage, each write must mark all of its rows as changed
        // across the whole width, or the panel keeps the old picture there.
        let g = Geometry::new(800, 1280, 3200, 32).unwrap();
        let layout = FbLayout {
            geometry: g,
            rotation: Rotation::Deg270,
        };
        for r in [
            Rect::new(0, 0, 1280, 800),
            Rect::new(10, 0, 1, 800),
            Rect::new(0, 0, 1, 1),
            Rect::new(1279, 799, 1, 1),
            Rect::new(300, 200, 170, 150),
        ] {
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
        let layout = FbLayout {
            geometry: g,
            rotation: Rotation::Deg0,
        };
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

    #[test]
    fn a_whole_frame_goes_to_the_screen_in_one_write() {
        let dir = std::env::temp_dir().join(format!("mpfree-ui-fb-frame-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("modes"), "U:4x3p-0\n").unwrap();
        fs::write(dir.join("stride"), "20\n").unwrap();
        fs::write(dir.join("bits_per_pixel"), "32\n").unwrap();
        fs::write(dir.join("pan"), "").unwrap();
        let fb = dir.join("fb");
        fs::write(&fb, vec![0u8; 60]).unwrap();
        let mut t = FbTarget::open(&fb, &dir, Rotation::Deg0).unwrap();
        let frame: Vec<u8> = (0..60u8).collect();
        t.present_frame(&frame).unwrap();
        assert_eq!(fs::read(&fb).unwrap(), frame);
        assert_eq!(fs::read_to_string(dir.join("pan")).unwrap(), "0,0\n");
        assert!(t.present_frame(&frame[..59]).is_err());
        // It never makes the file: no framebuffer, no screen.
        assert!(FbTarget::open(&dir.join("none"), &dir, Rotation::Deg0).is_err());
        assert!(!dir.join("none").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
