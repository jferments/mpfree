//! Plays mpfree-ui display lists ([`mpfree_ui::record`]) on the GPU with
//! OpenGL ES 2, straight into the panel's own orientation.
//!
//! The screens draw a landscape picture (1280 x 800 on the MPC One); the
//! panel is portrait (800 x 1280) and mounted turned. The software path
//! draws landscape and turns every pixel as it copies (`mpfree_ui::fb`).
//! Here the turn is part of the drawing: each call is drawn where its
//! pixels land on the panel, so the frame is already in the panel's order
//! and nothing is turned afterwards.
//!
//! The frame lives in a texture that keeps its pixels between frames, as
//! the software canvas does, so a frame's list only redraws what changed.
//!
//! [`Gpu::surfaceless`] opens EGL without any display (Mesa's surfaceless
//! platform), as the MPC's offscreen GPU tests did on 9 Nov; it reads
//! `libEGL.so.1` at run time, so nothing links against the system's GPU
//! libraries. [`Renderer::read_native`] reads the frame back, for the tests
//! and for copying to `/dev/fb0` until the panel can show the GPU's
//! buffers directly.
//!
//! The picture matches the software canvas's within a few levels per
//! colour channel where it blends (the GPU blends in floating point, the
//! software canvas in integers) and exactly elsewhere; the tests check it.

use std::collections::HashMap;
use std::ffi::c_void;

use glow::HasContext;
use khronos_egl as egl;
use mpfree_screen_log::Rotation;
use mpfree_ui::record::{Op, Store};
use mpfree_ui::{Color, Rect};

/// An EGL display and an OpenGL ES 2 context, current on this thread.
pub struct Gpu {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    pub gl: glow::Context,
}

/// Mesa's surfaceless platform (EGL_MESA_platform_surfaceless).
const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

impl Gpu {
    /// EGL on Mesa's surfaceless platform: a GPU with no window and no
    /// display. Fails (with the reason) if libEGL, the platform or an ES 2
    /// context is missing.
    pub fn surfaceless() -> Result<Gpu, String> {
        // SAFETY: loading the system's libEGL runs its initialisers, as any
        // program linked against it would.
        let lib = unsafe { libloading::Library::new("libEGL.so.1") }
            .map_err(|e| format!("libEGL.so.1: {e}"))?;
        // SAFETY: the library is EGL's; the functions are looked up by name.
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required_from(lib) }
            .map_err(|e| format!("EGL 1.5: {e}"))?;
        // SAFETY: the default display handle with no attributes.
        let display = unsafe {
            egl.get_platform_display(
                PLATFORM_SURFACELESS_MESA,
                egl::DEFAULT_DISPLAY,
                &[egl::ATTRIB_NONE],
            )
        }
        .map_err(|e| format!("the surfaceless platform: {e}"))?;
        egl.initialize(display)
            .map_err(|e| format!("eglInitialize: {e}"))?;
        egl.bind_api(egl::OPENGL_ES_API)
            .map_err(|e| format!("eglBindAPI: {e}"))?;
        let config = egl
            .choose_first_config(
                display,
                // Any surface type: there is no surface (the default asks
                // for a window, which surfaceless has none of).
                &[
                    egl::SURFACE_TYPE,
                    0,
                    egl::RENDERABLE_TYPE,
                    egl::OPENGL_ES2_BIT,
                    egl::NONE,
                ],
            )
            .map_err(|e| format!("eglChooseConfig: {e}"))?
            .ok_or("no OpenGL ES 2 configuration")?;
        let context = egl
            .create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE],
            )
            .map_err(|e| format!("eglCreateContext: {e}"))?;
        egl.make_current(display, None, None, Some(context))
            .map_err(|e| format!("eglMakeCurrent with no surface: {e}"))?;
        // SAFETY: the context is current on this thread; the loader returns
        // EGL's addresses for the GL functions.
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                egl.get_proc_address(name)
                    .map_or(std::ptr::null(), |f| f as *const c_void)
            })
        };
        Ok(Gpu {
            egl,
            display,
            context,
            gl,
        })
    }

    /// The driver's name for the GPU (GL_RENDERER) and its version.
    pub fn describe(&self) -> String {
        // SAFETY: plain queries on the current context.
        unsafe {
            format!(
                "{}; {}",
                self.gl.get_parameter_string(glow::RENDERER),
                self.gl.get_parameter_string(glow::VERSION)
            )
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}

/// Where a logical point lands on the panel: native = m * logical + t, for
/// points (a pixel's centre is at +0.5), matching `Geometry::native`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Turn {
    m: [f32; 4],
    t: [f32; 2],
}

impl Turn {
    fn new(rotation: Rotation, nw: f32, nh: f32) -> Turn {
        // (x, y) logical; rows of m: native x, native y.
        let (m, t) = match rotation {
            Rotation::Deg0 => ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0]),
            Rotation::Deg90 => ([0.0, -1.0, 1.0, 0.0], [nw, 0.0]),
            Rotation::Deg180 => ([-1.0, 0.0, 0.0, -1.0], [nw, nh]),
            Rotation::Deg270 => ([0.0, 1.0, -1.0, 0.0], [0.0, nh]),
        };
        Turn { m, t }
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let m = self.m;
        (
            m[0] * x + m[1] * y + self.t[0],
            m[2] * x + m[3] * y + self.t[1],
        )
    }

    /// The native rectangle a logical one covers.
    fn rect(&self, r: Rect) -> (i32, i32, i32, i32) {
        let (ax, ay) = self.apply(r.x as f32, r.y as f32);
        let (bx, by) = self.apply(r.right() as f32, r.bottom() as f32);
        let (x0, x1) = (ax.min(bx) as i32, ax.max(bx) as i32);
        let (y0, y1) = (ay.min(by) as i32, ay.max(by) as i32);
        (x0, y0, x1 - x0, y1 - y0)
    }
}

const SHAPE_VS: &str = r#"#version 100
uniform vec2 u_native;
uniform mat2 u_m;
uniform vec2 u_t;
attribute vec2 a_pos;
attribute vec4 a_rect;
attribute vec4 a_inner;
attribute vec2 a_rad;
attribute vec4 a_color;
varying vec4 v_rect;
varying vec4 v_inner;
varying vec2 v_rad;
varying vec4 v_color;
void main() {
    vec2 n = u_m * a_pos + u_t;
    gl_Position = vec4(n.x / u_native.x * 2.0 - 1.0, 1.0 - n.y / u_native.y * 2.0, 0.0, 1.0);
    v_rect = a_rect;
    v_inner = a_inner;
    v_rad = a_rad;
    v_color = a_color;
}
"#;

/// The software canvas's coverage (mpfree_ui::canvas::coverage), per pixel
/// centre in logical coordinates. `u_m` is a turn, so its inverse is its
/// transpose. A stroke is the outer shape less the inner one.
const SHAPE_FS: &str = r#"#version 100
precision highp float;
uniform vec2 u_native;
uniform mat2 u_m;
uniform vec2 u_t;
varying vec4 v_rect;
varying vec4 v_inner;
varying vec2 v_rad;
varying vec4 v_color;
float coverage(vec4 r, float radius, vec2 f) {
    if (r.z <= 0.0 || r.w <= 0.0) return 0.0;
    if (f.x < r.x || f.y < r.y || f.x > r.x + r.z || f.y > r.y + r.w) return 0.0;
    if (radius < 0.5) return 1.0;
    vec2 c = clamp(f, r.xy + radius, r.xy + r.zw - radius);
    return clamp(radius - length(f - c) + 0.5, 0.0, 1.0);
}
void main() {
    vec2 n = vec2(gl_FragCoord.x, u_native.y - gl_FragCoord.y);
    vec2 f = (n - u_t) * u_m;
    float cov = coverage(v_rect, v_rad.x, f);
    if (v_inner.z > 0.0) cov -= coverage(v_inner, v_rad.y, f);
    if (cov <= 0.0) discard;
    gl_FragColor = vec4(v_color.rgb, v_color.a * min(cov, 1.0));
}
"#;

const QUAD_VS: &str = r#"#version 100
uniform vec2 u_native;
uniform mat2 u_m;
uniform vec2 u_t;
attribute vec2 a_pos;
void main() {
    vec2 n = u_m * a_pos + u_t;
    gl_Position = vec4(n.x / u_native.x * 2.0 - 1.0, 1.0 - n.y / u_native.y * 2.0, 0.0, 1.0);
}
"#;

/// A mask (text, strokes) blended in `u_color`, or an image copied. The
/// texel is found from the logical pixel, so each pixel reads exactly its
/// own texel whatever the turn.
const QUAD_FS: &str = r#"#version 100
precision highp float;
uniform vec2 u_native;
uniform mat2 u_m;
uniform vec2 u_t;
uniform vec4 u_place;
uniform vec4 u_color;
uniform float u_image;
uniform sampler2D u_tex;
void main() {
    vec2 n = vec2(gl_FragCoord.x, u_native.y - gl_FragCoord.y);
    vec2 f = (n - u_t) * u_m;
    vec2 texel = floor(f - u_place.xy) + 0.5;
    vec4 s = texture2D(u_tex, texel / u_place.zw);
    if (u_image > 0.5) {
        gl_FragColor = vec4(s.rgb, 1.0);
    } else {
        if (s.r <= 0.0) discard;
        gl_FragColor = vec4(u_color.rgb, u_color.a * s.r);
    }
}
"#;

/// Floats per shape vertex: pos 2, rect 4, inner 4, radii 2, colour 4.
const SHAPE_FLOATS: usize = 16;

struct Program {
    id: glow::Program,
    native: Option<glow::UniformLocation>,
    m: Option<glow::UniformLocation>,
    t: Option<glow::UniformLocation>,
}

/// What a frame cost on the GPU's side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawStats {
    /// Draw calls made (a run of shapes is one).
    pub draws: usize,
    /// Masks and images uploaded as textures this frame.
    pub uploads: usize,
}

/// The GPU's copy of the frame, in the panel's orientation, and what it
/// needs to play lists into it.
pub struct Renderer {
    native: (i32, i32),
    logical: (i32, i32),
    turn: Turn,
    target: glow::Texture,
    fbo: glow::Framebuffer,
    shape: Program,
    quad: Program,
    place: Option<glow::UniformLocation>,
    color: Option<glow::UniformLocation>,
    image: Option<glow::UniformLocation>,
    vbo: glow::Buffer,
    verts: Vec<f32>,
    textures: HashMap<u32, (glow::Texture, i32, i32)>,
    clip: Rect,
    stats: DrawStats,
}

fn compile(gl: &glow::Context, vs: &str, fs: &str, attrs: &[&str]) -> Result<Program, String> {
    // SAFETY: plain GL calls on the current context with valid sources.
    unsafe {
        let p = gl.create_program()?;
        for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
            let s = gl.create_shader(kind)?;
            gl.shader_source(s, src);
            gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) {
                return Err(format!("shader: {}", gl.get_shader_info_log(s)));
            }
            gl.attach_shader(p, s);
            gl.delete_shader(s);
        }
        for (i, a) in attrs.iter().enumerate() {
            gl.bind_attrib_location(p, i as u32, a);
        }
        gl.link_program(p);
        if !gl.get_program_link_status(p) {
            return Err(format!("program: {}", gl.get_program_info_log(p)));
        }
        Ok(Program {
            id: p,
            native: gl.get_uniform_location(p, "u_native"),
            m: gl.get_uniform_location(p, "u_m"),
            t: gl.get_uniform_location(p, "u_t"),
        })
    }
}

fn rgba(c: Color) -> [f32; 4] {
    [
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a as f32 / 255.0,
    ]
}

impl Renderer {
    /// A black frame for a panel of `nw` x `nh` native pixels mounted
    /// turned by `rotation`.
    pub fn new(gpu: &Gpu, nw: i32, nh: i32, rotation: Rotation) -> Result<Renderer, String> {
        let gl = &gpu.gl;
        let logical = match rotation {
            Rotation::Deg0 | Rotation::Deg180 => (nw, nh),
            Rotation::Deg90 | Rotation::Deg270 => (nh, nw),
        };
        let shape = compile(
            gl,
            SHAPE_VS,
            SHAPE_FS,
            &["a_pos", "a_rect", "a_inner", "a_rad", "a_color"],
        )?;
        let quad = compile(gl, QUAD_VS, QUAD_FS, &["a_pos"])?;
        // SAFETY: plain GL calls on the current context.
        unsafe {
            let target = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(target));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                nw,
                nh,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32);
            }
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(target),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(format!(
                    "the frame's framebuffer is incomplete ({status:#x})"
                ));
            }
            gl.viewport(0, 0, nw, nh);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            let vbo = gl.create_buffer()?;
            let place = gl.get_uniform_location(quad.id, "u_place");
            let color = gl.get_uniform_location(quad.id, "u_color");
            let image = gl.get_uniform_location(quad.id, "u_image");
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            let r = Renderer {
                native: (nw, nh),
                logical,
                turn: Turn::new(rotation, nw as f32, nh as f32),
                target,
                fbo,
                shape,
                quad,
                place,
                color,
                image,
                vbo,
                verts: Vec::new(),
                textures: HashMap::new(),
                clip: Rect::new(0, 0, logical.0, logical.1),
                stats: DrawStats::default(),
            };
            for p in [&r.shape, &r.quad] {
                r.set_common(gl, p);
            }
            Ok(r)
        }
    }

    /// The logical (landscape) size the screens draw at.
    pub fn logical_size(&self) -> (i32, i32) {
        self.logical
    }

    /// The texture that holds the frame, in the panel's orientation (for
    /// showing it on the panel without reading it back, later).
    pub fn frame_texture(&self) -> glow::Texture {
        self.target
    }

    fn set_common(&self, gl: &glow::Context, p: &Program) {
        // SAFETY: uniforms of a linked program on the current context.
        unsafe {
            gl.use_program(Some(p.id));
            gl.uniform_2_f32(
                p.native.as_ref(),
                self.native.0 as f32,
                self.native.1 as f32,
            );
            // GLSL's mat2 is column-major: columns (m0, m2) and (m1, m3).
            let m = self.turn.m;
            gl.uniform_matrix_2_f32_slice(p.m.as_ref(), false, &[m[0], m[2], m[1], m[3]]);
            gl.uniform_2_f32(p.t.as_ref(), self.turn.t[0], self.turn.t[1]);
        }
    }

    /// Plays a frame's list into the frame. Masks and images are uploaded
    /// the first time a list names them; `dropped` (from
    /// `Store::take_changes`) frees the ones the store let go.
    pub fn play(&mut self, gpu: &Gpu, ops: &[Op], store: &Store, dropped: &[u32]) -> DrawStats {
        let gl = &gpu.gl;
        self.stats = DrawStats::default();
        for id in dropped {
            if let Some((t, _, _)) = self.textures.remove(id) {
                // SAFETY: a texture this renderer made.
                unsafe { gl.delete_texture(t) };
            }
        }
        // SAFETY: plain GL calls on the current context.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.viewport(0, 0, self.native.0, self.native.1);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.enable(glow::SCISSOR_TEST);
        }
        self.clip = Rect::new(0, 0, self.logical.0, self.logical.1);
        self.scissor(gl);
        for op in ops {
            match *op {
                Op::Clip(r) => {
                    self.flush(gl);
                    self.clip = r.intersect(Rect::new(0, 0, self.logical.0, self.logical.1));
                    self.scissor(gl);
                }
                Op::Fill(r, c) => self.push_shape(r, 0, None, 0, c),
                Op::FillRound { r, radius, c } => {
                    let rad = radius.clamp(0, r.w.min(r.h) / 2);
                    self.push_shape(r, rad, None, 0, c);
                }
                Op::StrokeRound {
                    r,
                    radius,
                    width,
                    c,
                } => {
                    let rad = radius.clamp(0, r.w.min(r.h) / 2);
                    let width = width.max(1);
                    let inner = r.inset(width);
                    let irad = (rad - width).max(0);
                    let inner = if inner.w > 0 && inner.h > 0 {
                        Some(inner)
                    } else {
                        None
                    };
                    self.push_shape(r, rad, inner, irad, c);
                }
                Op::Mask { x, y, id, c } => {
                    self.flush(gl);
                    if let Some(m) = store.mask(id) {
                        let (w, h) = (m.w as i32, m.h as i32);
                        if w > 0 && h > 0 {
                            let tex =
                                self.texture(gl, id, || (w, h, glow::LUMINANCE, m.data.clone()));
                            self.draw_quad(gl, tex, Rect::new(x, y, w, h), Some(c));
                        }
                    }
                }
                Op::Image { x, y, id } => {
                    self.flush(gl);
                    if let Some(img) = store.image(id) {
                        let tex = self.texture(gl, id, || {
                            let mut b = Vec::with_capacity(img.px.len() * 4);
                            for p in &img.px {
                                b.extend_from_slice(&[
                                    (p >> 16) as u8,
                                    (p >> 8) as u8,
                                    *p as u8,
                                    255,
                                ]);
                            }
                            (img.w, img.h, glow::RGBA, b)
                        });
                        self.draw_quad(gl, tex, Rect::new(x, y, img.w, img.h), None);
                    }
                }
            }
        }
        self.flush(gl);
        self.stats
    }

    fn scissor(&self, gl: &glow::Context) {
        let (x, y, w, h) = self.turn.rect(self.clip);
        // GL's window origin is at the bottom left; native rows run down.
        // SAFETY: a plain GL call.
        unsafe { gl.scissor(x, self.native.1 - y - h, w.max(0), h.max(0)) };
    }

    fn push_shape(&mut self, r: Rect, rad: i32, inner: Option<Rect>, irad: i32, c: Color) {
        let area = r.intersect(self.clip);
        if area.w <= 0 || area.h <= 0 {
            return;
        }
        let rect = [r.x as f32, r.y as f32, r.w as f32, r.h as f32];
        let inner = inner.map_or([0.0; 4], |i| {
            [i.x as f32, i.y as f32, i.w as f32, i.h as f32]
        });
        let col = rgba(c);
        let (x0, y0) = (area.x as f32, area.y as f32);
        let (x1, y1) = (area.right() as f32, area.bottom() as f32);
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y0), (x1, y1), (x0, y1)] {
            self.verts.extend_from_slice(&[x, y]);
            self.verts.extend_from_slice(&rect);
            self.verts.extend_from_slice(&inner);
            self.verts.extend_from_slice(&[rad as f32, irad as f32]);
            self.verts.extend_from_slice(&col);
        }
    }

    /// Draws the shapes gathered since the last flush, in one call.
    fn flush(&mut self, gl: &glow::Context) {
        if self.verts.is_empty() {
            return;
        }
        let stride = (SHAPE_FLOATS * 4) as i32;
        // SAFETY: the buffer holds whole vertices of SHAPE_FLOATS floats.
        unsafe {
            gl.use_program(Some(self.shape.id));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let bytes =
                std::slice::from_raw_parts(self.verts.as_ptr() as *const u8, self.verts.len() * 4);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
            let mut off = 0;
            for (i, n) in [2, 4, 4, 2, 4].into_iter().enumerate() {
                gl.enable_vertex_attrib_array(i as u32);
                gl.vertex_attrib_pointer_f32(i as u32, n, glow::FLOAT, false, stride, off);
                off += n * 4;
            }
            gl.draw_arrays(glow::TRIANGLES, 0, (self.verts.len() / SHAPE_FLOATS) as i32);
            for i in 0..5 {
                gl.disable_vertex_attrib_array(i);
            }
        }
        self.verts.clear();
        self.stats.draws += 1;
    }

    fn texture(
        &mut self,
        gl: &glow::Context,
        id: u32,
        make: impl FnOnce() -> (i32, i32, u32, Vec<u8>),
    ) -> (glow::Texture, i32, i32) {
        if let Some(t) = self.textures.get(&id) {
            return *t;
        }
        let (w, h, format, data) = make();
        // SAFETY: `data` holds w * h texels of `format`, rows unpadded.
        let t = unsafe {
            let t = gl.create_texture().expect("a texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32);
            }
            for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32);
            }
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                format as i32,
                w,
                h,
                0,
                format,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&data)),
            );
            t
        };
        self.stats.uploads += 1;
        self.textures.insert(id, (t, w, h));
        (t, w, h)
    }

    /// One mask (blended in `color`) or image (copied) at `place`.
    fn draw_quad(
        &mut self,
        gl: &glow::Context,
        tex: (glow::Texture, i32, i32),
        place: Rect,
        color: Option<Color>,
    ) {
        let area = place.intersect(self.clip);
        if area.w <= 0 || area.h <= 0 {
            return;
        }
        let (x0, y0) = (area.x as f32, area.y as f32);
        let (x1, y1) = (area.right() as f32, area.bottom() as f32);
        let v = [x0, y0, x1, y0, x1, y1, x0, y0, x1, y1, x0, y1];
        // SAFETY: plain GL calls with a 12-float vertex array.
        unsafe {
            gl.use_program(Some(self.quad.id));
            gl.uniform_4_f32(
                self.place.as_ref(),
                place.x as f32,
                place.y as f32,
                tex.1 as f32,
                tex.2 as f32,
            );
            let c = rgba(color.unwrap_or(Color::rgb(0)));
            gl.uniform_4_f32(self.color.as_ref(), c[0], c[1], c[2], c[3]);
            gl.uniform_1_f32(self.image.as_ref(), if color.is_none() { 1.0 } else { 0.0 });
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex.0));
            if color.is_none() {
                gl.disable(glow::BLEND);
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
            gl.disable_vertex_attrib_array(0);
            if color.is_none() {
                gl.enable(glow::BLEND);
            }
        }
        self.stats.draws += 1;
    }

    /// The frame as the panel takes it: native rows top to bottom, one
    /// XRGB8888 word per pixel. Waits for the GPU to finish.
    pub fn read_native(&self, gpu: &Gpu) -> Vec<u32> {
        let gl = &gpu.gl;
        let (w, h) = (self.native.0 as usize, self.native.1 as usize);
        let mut bytes = vec![0u8; w * h * 4];
        // SAFETY: `bytes` holds the whole frame as RGBA bytes.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut bytes)),
            );
        }
        let mut out = vec![0u32; w * h];
        for ny in 0..h {
            // GL's row 0 is the bottom one.
            let src = &bytes[(h - 1 - ny) * w * 4..(h - ny) * w * 4];
            for (nx, p) in src.chunks_exact(4).enumerate() {
                out[ny * w + nx] = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
            }
        }
        out
    }

    /// Waits for the GPU to finish what it was given.
    pub fn finish(&self, gpu: &Gpu) {
        // SAFETY: a plain GL call.
        unsafe { gpu.gl.finish() };
    }
}

#[cfg(test)]
mod tests;
