//! Drawing straight into the buffers the panel shows, with no read-back.
//!
//! The GPU draws a frame's list into a buffer the display controller can
//! scan out (a dma-buf, imported into EGL as an image and bound as a
//! texture), then the display shows that buffer from the next vertical
//! blank. With two buffers, one is on the panel while the other is drawn,
//! so nothing is ever seen half drawn, and nothing is read back or copied
//! by the CPU.
//!
//! Lists only redraw what changed, but each buffer last held an older
//! frame: before a list is drawn into a buffer, the rectangles it missed
//! (what the frames since then changed) are copied into it from the buffer
//! on the panel, which holds the newest frame. [`Scanout`] keeps those
//! rectangles per buffer. The copy and the list go in one pass, cut to the
//! rectangle around both, so a small change costs the GPU little.
//!
//! [`Display`] is what shows the buffers: [`crate::kms::KmsDisplay`] on the
//! MPC, [`Offscreen`] for tests and dry runs.

use std::ffi::c_void;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mpfree_ui::record::{Op, Store};
use mpfree_ui::Rect;

use crate::{list_damage, Damage, DrawStats, Gpu, Renderer, Target};

/// Shows one of the buffers on the panel.
pub trait Display {
    /// Shows buffer `index` (from the next vertical blank, once the first
    /// one is up). Returns once asked; [`Display::wait`] waits for it.
    fn show(&mut self, index: usize) -> Result<(), String>;
    /// Waits until the buffer shown last is on the panel, so the one
    /// before it can be drawn into again.
    fn wait(&mut self) -> Result<(), String>;
}

/// A display that shows nothing: it remembers which buffer it was asked to
/// show. For tests, and to run the renderer process without a panel.
#[derive(Debug, Default)]
pub struct Offscreen {
    pub shown: Vec<usize>,
}

impl Display for Offscreen {
    fn show(&mut self, index: usize) -> Result<(), String> {
        self.shown.push(index);
        Ok(())
    }

    fn wait(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// What showing one frame cost.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Presented {
    pub draw: DrawStats,
    /// The buffer it went into.
    pub buffer: usize,
    /// Waiting for the last flip, so this buffer was free.
    pub waited: Duration,
    /// Building and sending the GPU's work (CPU time, about).
    pub drawn: Duration,
    /// Waiting for the GPU to finish it.
    pub finished: Duration,
}

/// The buffers the panel shows in turn, and what each one has missed.
pub struct Scanout {
    targets: Vec<Target>,
    /// Native rectangles each buffer has missed since it was last drawn.
    stale: Vec<Damage>,
    /// The buffer on the panel (or asked for), None before the first frame.
    front: Option<usize>,
    pending: bool,
    native: (i32, i32),
    logical: (i32, i32),
}

impl Scanout {
    /// Takes `targets` (at least two, each the renderer's native size) and
    /// paints them black, like the renderer's own first frame.
    pub fn new(gpu: &Gpu, r: &Renderer, targets: Vec<Target>) -> Result<Scanout, String> {
        if targets.len() < 2 {
            return Err(format!(
                "{} buffers for the panel; showing one while drawing another needs two",
                targets.len()
            ));
        }
        let native = r.native_size();
        for t in &targets {
            t.clear(gpu, native.0, native.1);
        }
        r.finish(gpu);
        Ok(Scanout {
            stale: vec![Damage::default(); targets.len()],
            targets,
            front: None,
            pending: false,
            native,
            logical: r.logical_size(),
        })
    }

    pub fn targets(&self) -> &[Target] {
        &self.targets
    }

    /// The buffer that holds the newest frame (on the panel, or asked for).
    pub fn front(&self) -> Option<usize> {
        self.front
    }

    /// Draws a frame's list into the next buffer and shows it. A list that
    /// draws nothing shows nothing (None), but still frees what `dropped`
    /// names.
    pub fn present(
        &mut self,
        gpu: &Gpu,
        r: &mut Renderer,
        display: &mut dyn Display,
        ops: &[Op],
        store: &Store,
        dropped: &[u32],
    ) -> Result<Option<Presented>, String> {
        if list_damage(ops, store, self.logical.0, self.logical.1).is_empty() {
            r.drop_items(gpu, dropped);
            return Ok(None);
        }
        let t = Instant::now();
        if self.pending {
            display.wait()?;
            self.pending = false;
        }
        let waited = t.elapsed();
        let back = self.front.map_or(0, |f| (f + 1) % self.targets.len());
        let missed: Vec<Rect> = self.stale[back].rects().to_vec();
        let refresh = self
            .front
            .filter(|_| !missed.is_empty())
            .map(|f| (self.targets[f].texture, &missed[..]));
        let t = Instant::now();
        let draw = r.play_on(gpu, self.targets[back], refresh, ops, store, dropped);
        let drawn = t.elapsed();
        r.finish(gpu);
        let finished = t.elapsed() - drawn;
        let changed: Vec<Rect> = r.damage().to_vec();
        for (i, s) in self.stale.iter_mut().enumerate() {
            if i == back {
                s.clear();
            } else {
                s.extend(&changed);
            }
        }
        display.show(back)?;
        self.pending = true;
        self.front = Some(back);
        Ok(Some(Presented {
            draw,
            buffer: back,
            waited,
            drawn,
            finished,
        }))
    }

    /// The newest frame, as the panel takes it (native rows, XRGB words).
    pub fn read_front(&self, gpu: &Gpu) -> Option<Vec<u32>> {
        self.front
            .map(|f| self.targets[f].read(gpu, self.native.0, self.native.1))
    }
}

/// Pixels where a screen buffer's own memory (`bytes`: rows `pitch` bytes
/// apart, XRGB8888, as the display controller reads it) differs from what
/// the GPU drew into `target`, the same buffer seen through EGL. Zero means
/// the panel would show exactly the GPU's picture, right way up.
pub fn buffer_differs(
    gpu: &Gpu,
    r: &Renderer,
    target: Target,
    bytes: &[u8],
    pitch: usize,
) -> usize {
    let (nw, _) = r.native_size();
    let want = r.read_target(gpu, target);
    let mut wrong = 0usize;
    for (ny, row) in want.chunks_exact(nw as usize).enumerate() {
        let Some(line) = bytes.get(ny * pitch..ny * pitch + nw as usize * 4) else {
            wrong += row.len();
            continue;
        };
        for (p, b) in row.iter().zip(line.as_chunks::<4>().0) {
            wrong += (u32::from_le_bytes(*b) & 0xFF_FFFF != *p) as usize;
        }
    }
    wrong
}

/// A test card for checking a screen buffer: bands of distinct colours and
/// a rounded box, so a buffer read upside down, turned, shifted or in
/// another byte order cannot pass for it.
pub fn test_card(w: i32, h: i32) -> mpfree_ui::record::Recorder {
    use mpfree_ui::{Canvas, Color};
    let mut rec = mpfree_ui::record::Recorder::new(w, h);
    rec.begin_frame();
    let colours = [
        0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0x00FFFF, 0xFF00FF, 0xFFFFFF, 0x808080,
    ];
    let band = (w / colours.len() as i32).max(1);
    for (i, c) in colours.iter().enumerate() {
        rec.fill_rect(Rect::new(i as i32 * band, 0, band, h), Color::rgb(*c));
    }
    rec.fill_rect(Rect::new(0, 0, w, h / 8), Color::rgb(0x102030));
    rec.fill_round_rect(
        Rect::new(w / 3, h / 3, w / 3, h / 3),
        24,
        Color::rgb(0xF0A020).with_alpha(200),
    );
    rec
}

// ------------------------------------------------------------ dma-bufs into EGL

/// EGL_LINUX_DMA_BUF_EXT and its attributes (EGL_EXT_image_dma_buf_import).
const LINUX_DMA_BUF: u32 = 0x3270;
const LINUX_DRM_FOURCC: i32 = 0x3271;
const DMA_BUF_PLANE0_FD: i32 = 0x3272;
const DMA_BUF_PLANE0_OFFSET: i32 = 0x3273;
const DMA_BUF_PLANE0_PITCH: i32 = 0x3274;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_NONE: i32 = 0x3038;

type CreateImageKhr = unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    u32,
    *mut c_void,
    *const i32,
) -> *mut c_void;
type DestroyImageKhr = unsafe extern "system" fn(*mut c_void, *mut c_void) -> u32;
type ImageTargetTexture2dOes = unsafe extern "system" fn(u32, *mut c_void);

/// What importing dma-bufs needs, and whether the driver has it.
pub fn import_support(gpu: &Gpu) -> Result<(), String> {
    for ext in ["EGL_KHR_image_base", "EGL_EXT_image_dma_buf_import"] {
        if !gpu.has_egl_extension(ext) {
            return Err(format!("EGL has no {ext}"));
        }
    }
    if !gpu.has_gl_extension("GL_OES_EGL_image") {
        return Err("OpenGL ES has no GL_OES_EGL_image".into());
    }
    Ok(())
}

/// A panel buffer the GPU draws into: an EGL image over a dma-buf, bound
/// to a texture and a framebuffer.
pub struct Imported {
    image: *mut c_void,
    pub target: Target,
}

/// Imports a `w` x `h` dma-buf (one plane, `pitch` bytes a row, DRM format
/// `fourcc`) as a target the renderer can draw into. The dma-buf can be
/// closed afterwards; the image keeps the buffer.
pub fn import_dmabuf(
    gpu: &Gpu,
    fd: BorrowedFd<'_>,
    w: i32,
    h: i32,
    pitch: u32,
    fourcc: u32,
) -> Result<Imported, String> {
    import_support(gpu)?;
    let create = gpu
        .egl
        .get_proc_address("eglCreateImageKHR")
        .ok_or("EGL has no eglCreateImageKHR")?;
    let bind = gpu
        .egl
        .get_proc_address("glEGLImageTargetTexture2DOES")
        .ok_or("EGL has no glEGLImageTargetTexture2DOES")?;
    // SAFETY: EGL_KHR_image_base and GL_OES_EGL_image define the functions
    // with these types.
    let create: CreateImageKhr = unsafe { std::mem::transmute(create) };
    let bind: ImageTargetTexture2dOes = unsafe { std::mem::transmute(bind) };
    let attrs = [
        EGL_WIDTH,
        w,
        EGL_HEIGHT,
        h,
        LINUX_DRM_FOURCC,
        fourcc as i32,
        DMA_BUF_PLANE0_FD,
        fd.as_raw_fd(),
        DMA_BUF_PLANE0_OFFSET,
        0,
        DMA_BUF_PLANE0_PITCH,
        pitch as i32,
        EGL_NONE,
    ];
    // SAFETY: the display is initialised; a dma-buf import takes no
    // context and no client buffer; `attrs` ends with EGL_NONE.
    let image = unsafe {
        create(
            gpu.display.as_ptr(),
            std::ptr::null_mut(),
            LINUX_DMA_BUF,
            std::ptr::null_mut(),
            attrs.as_ptr(),
        )
    };
    if image.is_null() {
        return Err(format!(
            "eglCreateImageKHR refused the screen buffer ({w}x{h}, pitch {pitch}): {}",
            gpu.egl
                .get_error()
                .map_or("no error given".into(), |e| e.to_string())
        ));
    }
    let gl = &gpu.gl;
    let texture = {
        use glow::HasContext;
        // SAFETY: a texture bound to the image just made, on the current
        // context.
        unsafe {
            let t = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            bind(glow::TEXTURE_2D, image);
            let err = gl.get_error();
            if err != glow::NO_ERROR {
                gl.delete_texture(t);
                destroy_image(gpu, image);
                return Err(format!(
                    "binding the screen buffer to a texture failed (GL error {err:#x})"
                ));
            }
            t
        }
    };
    match Target::for_texture(gpu, texture) {
        Ok(target) => Ok(Imported { image, target }),
        Err(e) => {
            use glow::HasContext;
            // SAFETY: the texture made above.
            unsafe { gl.delete_texture(texture) };
            destroy_image(gpu, image);
            Err(format!("the screen buffer can't be drawn into: {e}"))
        }
    }
}

fn destroy_image(gpu: &Gpu, image: *mut c_void) {
    if let Some(f) = gpu.egl.get_proc_address("eglDestroyImageKHR") {
        // SAFETY: EGL_KHR_image_base defines the function with this type.
        let destroy: DestroyImageKhr = unsafe { std::mem::transmute(f) };
        // SAFETY: an image made on this display.
        unsafe { destroy(gpu.display.as_ptr(), image) };
    }
}

impl Imported {
    /// Frees the texture, the framebuffer and the image.
    pub fn release(self, gpu: &Gpu) {
        self.target.delete(gpu, true);
        destroy_image(gpu, self.image);
    }
}

// ------------------------------------------------------------ stopping cleanly

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// SIGINT, SIGTERM and SIGHUP (a closed SSH session) ask the program to
/// stop at the next frame, so it puts the screen back before it exits;
/// SIGPIPE is ignored, so a closed pipe is an error to handle, not death.
pub fn stop_on_signals() {
    // SAFETY: the handler only stores to an atomic, which is async-signal
    // safe.
    unsafe {
        for s in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            libc::signal(s, on_signal as *const () as libc::sighandler_t);
        }
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

/// True once one of those signals came.
pub fn stopping() -> bool {
    STOP.load(Ordering::SeqCst)
}
