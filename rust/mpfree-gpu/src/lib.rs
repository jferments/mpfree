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
    egl: egl::DynamicInstance<egl::EGL1_4>,
    display: egl::Display,
    context: egl::Context,
    /// EGL's version and vendor, as the display reports them.
    egl_version: String,
    pub gl: glow::Context,
    /// Kept loaded while `gl` holds its functions.
    _gles: Option<libloading::Library>,
}

/// Mesa's surfaceless platform (EGL_MESA_platform_surfaceless).
const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

/// EGL_OPENGL_ES3_BIT and EGL_CONTEXT_MINOR_VERSION (EGL_KHR_create_context).
const OPENGL_ES3_BIT: egl::Int = 0x40;
const CONTEXT_MINOR_VERSION: egl::Int = 0x30FB;

/// eglGetPlatformDisplayEXT (EGL_EXT_platform_base).
type GetPlatformDisplayExt =
    unsafe extern "system" fn(egl::Enum, *mut c_void, *const egl::Int) -> egl::EGLDisplay;

impl Gpu {
    /// EGL on Mesa's surfaceless platform: a GPU with no window and no
    /// display. Fails (with the reason) if libEGL, the platform or an ES 2
    /// context is missing.
    ///
    /// It takes the same path as the MPC's offscreen tests of 9 Nov
    /// (`tools/mpc-one/gpu-qualification/mpfree-egl-smoke.c`), which is
    /// what the MPC is known to support: EGL 1.4 (the MPC's Mesa reports
    /// 1.4), the display from `eglGetPlatformDisplayEXT`, and the GL
    /// functions from `libGLESv2.so.2` (EGL's own lookup only if that
    /// library is missing).
    pub fn surfaceless() -> Result<Gpu, String> {
        Gpu::surfaceless_es(2, 0)
    }

    /// As [`Gpu::surfaceless`], with an OpenGL ES `major`.`minor` context
    /// (3.1 for compute shaders; a minor version needs EGL_KHR_create_context).
    pub fn surfaceless_es(major: i32, minor: i32) -> Result<Gpu, String> {
        // SAFETY: loading the system's libEGL runs its initialisers, as any
        // program linked against it would.
        let lib = unsafe { libloading::Library::new("libEGL.so.1") }
            .map_err(|e| format!("libEGL.so.1: {e}"))?;
        // SAFETY: the library is EGL's; the functions are looked up by name.
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_4>::load_required_from(lib) }
            .map_err(|e| format!("EGL 1.4: {e}"))?;
        let client = egl
            .query_string(None, egl::EXTENSIONS)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        for ext in ["EGL_EXT_platform_base", "EGL_MESA_platform_surfaceless"] {
            if !client.split(' ').any(|e| e == ext) {
                return Err(format!("EGL has no {ext}"));
            }
        }
        let get_platform_display = egl
            .get_proc_address("eglGetPlatformDisplayEXT")
            .ok_or("EGL has no eglGetPlatformDisplayEXT")?;
        // SAFETY: EGL_EXT_platform_base defines the function with this type.
        let get_platform_display: GetPlatformDisplayExt =
            unsafe { std::mem::transmute(get_platform_display) };
        // SAFETY: the surfaceless platform takes no native display and no
        // attributes.
        let raw = unsafe {
            get_platform_display(
                PLATFORM_SURFACELESS_MESA,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if raw.is_null() {
            return Err("the surfaceless platform gave no display".into());
        }
        // SAFETY: a display EGL just returned.
        let display = unsafe { egl::Display::from_ptr(raw) };
        let (egl_major, egl_minor) = egl
            .initialize(display)
            .map_err(|e| format!("eglInitialize: {e}"))?;
        let vendor = egl
            .query_string(Some(display), egl::VENDOR)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let egl_version = format!("EGL {egl_major}.{egl_minor} ({vendor})");
        let display_ext = egl
            .query_string(Some(display), egl::EXTENSIONS)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
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
                    if major >= 3 {
                        OPENGL_ES3_BIT
                    } else {
                        egl::OPENGL_ES2_BIT
                    },
                    egl::NONE,
                ],
            )
            .map_err(|e| format!("eglChooseConfig: {e}"))?
            .ok_or_else(|| format!("no OpenGL ES {major} configuration"))?;
        let mut attrs = vec![egl::CONTEXT_CLIENT_VERSION, major];
        if minor > 0 {
            if !display_ext
                .split(' ')
                .any(|e| e == "EGL_KHR_create_context")
            {
                return Err(format!(
                    "OpenGL ES {major}.{minor} needs EGL_KHR_create_context"
                ));
            }
            attrs.extend([CONTEXT_MINOR_VERSION, minor]);
        }
        attrs.push(egl::NONE);
        let context = egl
            .create_context(display, config, None, &attrs)
            .map_err(|e| format!("eglCreateContext (OpenGL ES {major}.{minor}): {e}"))?;
        egl.make_current(display, None, None, Some(context))
            .map_err(|e| format!("eglMakeCurrent with no surface: {e}"))?;
        // SAFETY: loading the system's GLES library runs its initialisers, as
        // any program linked against it would.
        let gles = unsafe { libloading::Library::new("libGLESv2.so.2") }.ok();
        // SAFETY: the context is current on this thread; each name is looked
        // up in libGLESv2 (as the 9 Nov tests did), or else through EGL.
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                let from_lib = gles.as_ref().and_then(|lib| {
                    let mut sym = name.as_bytes().to_vec();
                    sym.push(0);
                    lib.get::<*const c_void>(&sym).ok().map(|s| *s)
                });
                from_lib
                    .filter(|p| !p.is_null())
                    .or_else(|| egl.get_proc_address(name).map(|f| f as *const c_void))
                    .unwrap_or(std::ptr::null())
            })
        };
        Ok(Gpu {
            egl,
            display,
            context,
            egl_version,
            gl,
            _gles: gles,
        })
    }

    /// The driver's name for the GPU (GL_RENDERER), its GL version and EGL's.
    pub fn describe(&self) -> String {
        // SAFETY: plain queries on the current context.
        unsafe {
            format!(
                "{}; {}; {}",
                self.gl.get_parameter_string(glow::RENDERER),
                self.gl.get_parameter_string(glow::VERSION),
                self.egl_version
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
}

const BATCH_VS: &str = r#"#version 100
uniform vec2 u_native;
uniform mat2 u_m;
uniform vec2 u_t;
attribute vec2 a_pos;
attribute vec4 a_rect;
attribute vec4 a_inner;
attribute vec3 a_rad;
attribute vec4 a_color;
varying vec4 v_rect;
varying vec4 v_inner;
varying vec3 v_rad;
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

/// Shapes and masks in one draw. A shape (`v_rad.z` 0) takes the software
/// canvas's coverage (mpfree_ui::canvas::coverage), per pixel centre in
/// logical coordinates; a stroke is the outer shape less the inner one. A
/// mask (`v_rad.z` 1) sits at `v_rect.xy` and reads its coverage from the
/// atlas at `v_inner.xy`, each pixel exactly its own texel whatever the
/// turn. `u_m` is a turn, so its inverse is its transpose.
const BATCH_FS: &str = r#"#version 100
precision highp float;
uniform vec2 u_native;
uniform mat2 u_m;
uniform vec2 u_t;
uniform vec2 u_atlas_size;
uniform sampler2D u_atlas;
varying vec4 v_rect;
varying vec4 v_inner;
varying vec3 v_rad;
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
    float cov;
    if (v_rad.z > 0.5) {
        vec2 texel = floor(f - v_rect.xy) + 0.5 + v_inner.xy;
        cov = texture2D(u_atlas, texel / u_atlas_size).r;
    } else {
        cov = coverage(v_rect, v_rad.x, f);
        if (v_inner.z > 0.0) cov -= coverage(v_inner, v_rad.y, f);
    }
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

/// An image copied, or a mask too big for the atlas blended in `u_color`,
/// each in its own texture and draw call. The texel is found from the
/// logical pixel, so each pixel reads exactly its own texel whatever the
/// turn.
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

/// One corner of a shape or mask in the batch (see `BATCH_VS`): 56 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 2],
    /// A shape's rectangle, or where a mask sits (x, y, w, h).
    rect: [f32; 4],
    /// A stroke's inner rectangle, or the mask's place in the atlas (x, y).
    inner: [f32; 4],
    /// The radii (outer, inner) and the kind: 0 shape, 1 mask.
    rad: [f32; 3],
    color: [u8; 4],
}

/// The batch's attributes: name, components, GL type, normalised.
const VERTEX_ATTRS: [(&str, i32, u32, bool); 5] = [
    ("a_pos", 2, glow::FLOAT, false),
    ("a_rect", 4, glow::FLOAT, false),
    ("a_inner", 4, glow::FLOAT, false),
    ("a_rad", 3, glow::FLOAT, false),
    ("a_color", 4, glow::UNSIGNED_BYTE, true),
];

/// Quads in one draw call: four corners each, so the 16-bit indices
/// reach every corner.
const MAX_QUADS: usize = 16384;

/// The masks' shared texture, at most (it is smaller where the GPU's
/// textures are): 2 MiB, four times the mask pixels of the busiest of the
/// 59 sample pages.
const ATLAS_SIZE: (i32, i32) = (2048, 1024);

struct Program {
    id: glow::Program,
    native: Option<glow::UniformLocation>,
    m: Option<glow::UniformLocation>,
    t: Option<glow::UniformLocation>,
}

/// What a frame cost on the GPU's side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawStats {
    /// Draw calls made. A run of shapes and masks is one; each image is
    /// one more.
    pub draws: usize,
    /// Masks and images uploaded this frame.
    pub uploads: usize,
    /// Times the atlas was full and was cleared, so that the masks still in
    /// use were uploaded again (a cost, not a fault; counted to be seen).
    pub atlas_cleared: usize,
    /// Masks too big for the atlas, drawn alone in their own textures.
    pub masks_alone: usize,
}

/// The masks' shared texture, filled in shelves: a run of text and shapes
/// is one draw call. When it is full the renderer draws what is waiting,
/// clears it, and uploads each mask again as a list uses it.
struct Atlas {
    tex: glow::Texture,
    size: (i32, i32),
    /// Each shelf's top, height and first free column.
    shelves: Vec<(i32, i32, i32)>,
    /// Where each mask is.
    at: HashMap<u32, (i32, i32)>,
}

impl Atlas {
    /// Room for a `w` x `h` mask, or None when the atlas is full (or the
    /// mask is bigger than the atlas). Shelf heights go up in steps of 8,
    /// so text of about the same size shares shelves.
    fn place(&mut self, w: i32, h: i32) -> Option<(i32, i32)> {
        let step = (h + 7) & !7;
        let mut best: Option<usize> = None;
        for (i, &(_, sh, x)) in self.shelves.iter().enumerate() {
            let fits = sh >= h && sh <= step + 8 && x + w <= self.size.0;
            if fits && best.is_none_or(|b| sh < self.shelves[b].1) {
                best = Some(i);
            }
        }
        let i = match best {
            Some(i) => i,
            None => {
                let top = self.shelves.last().map_or(0, |&(y, sh, _)| y + sh);
                let sh = step.min(self.size.1 - top);
                if w > self.size.0 || sh < h {
                    return None;
                }
                self.shelves.push((top, sh, 0));
                self.shelves.len() - 1
            }
        };
        let shelf = &mut self.shelves[i];
        let at = (shelf.2, shelf.0);
        shelf.2 += w;
        Some(at)
    }

    fn clear(&mut self) {
        self.shelves.clear();
        self.at.clear();
    }
}

/// The GPU's copy of the frame, in the panel's orientation, and what it
/// needs to play lists into it.
pub struct Renderer {
    native: (i32, i32),
    logical: (i32, i32),
    turn: Turn,
    target: glow::Texture,
    fbo: glow::Framebuffer,
    batch: Program,
    quad: Program,
    place: Option<glow::UniformLocation>,
    color: Option<glow::UniformLocation>,
    image: Option<glow::UniformLocation>,
    vbo: glow::Buffer,
    /// The corners' order, two triangles per quad, for every batch.
    ibo: glow::Buffer,
    verts: Vec<Vertex>,
    atlas: Atlas,
    /// Images and masks too big for the atlas, one texture each.
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
        Renderer::with_atlas(gpu, nw, nh, rotation, ATLAS_SIZE)
    }

    /// As `new`, with an atlas of at most `atlas` texels (the tests make a
    /// small one to fill it).
    fn with_atlas(
        gpu: &Gpu,
        nw: i32,
        nh: i32,
        rotation: Rotation,
        atlas: (i32, i32),
    ) -> Result<Renderer, String> {
        let gl = &gpu.gl;
        let logical = match rotation {
            Rotation::Deg0 | Rotation::Deg180 => (nw, nh),
            Rotation::Deg90 | Rotation::Deg270 => (nh, nw),
        };
        let names: Vec<&str> = VERTEX_ATTRS.iter().map(|a| a.0).collect();
        let batch = compile(gl, BATCH_VS, BATCH_FS, &names)?;
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
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            // The atlas, as big as asked where the GPU allows.
            let max = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE).max(64);
            let size = (atlas.0.min(max), atlas.1.min(max));
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32);
            }
            for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32);
            }
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::LUMINANCE as i32,
                size.0,
                size.1,
                0,
                glow::LUMINANCE,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            // Two triangles per quad, corners 0 1 2 and 0 2 3.
            let mut order = Vec::with_capacity(MAX_QUADS * 6);
            for q in 0..MAX_QUADS as u16 {
                let c = q * 4;
                order.extend_from_slice(&[c, c + 1, c + 2, c, c + 2, c + 3]);
            }
            let ibo = gl.create_buffer()?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ibo));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                std::slice::from_raw_parts(order.as_ptr() as *const u8, order.len() * 2),
                glow::STATIC_DRAW,
            );
            let vbo = gl.create_buffer()?;
            let place = gl.get_uniform_location(quad.id, "u_place");
            let color = gl.get_uniform_location(quad.id, "u_color");
            let image = gl.get_uniform_location(quad.id, "u_image");
            let r = Renderer {
                native: (nw, nh),
                logical,
                turn: Turn::new(rotation, nw as f32, nh as f32),
                target,
                fbo,
                batch,
                quad,
                place,
                color,
                image,
                vbo,
                ibo,
                verts: Vec::new(),
                atlas: Atlas {
                    tex,
                    size,
                    shelves: Vec::new(),
                    at: HashMap::new(),
                },
                textures: HashMap::new(),
                clip: Rect::new(0, 0, logical.0, logical.1),
                stats: DrawStats::default(),
            };
            for p in [&r.batch, &r.quad] {
                r.set_common(gl, p);
            }
            gl.use_program(Some(r.batch.id));
            gl.uniform_2_f32(
                gl.get_uniform_location(r.batch.id, "u_atlas_size").as_ref(),
                size.0 as f32,
                size.1 as f32,
            );
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
    ///
    /// Shapes and masks gather into one draw call until an image (or a
    /// mask too big for the atlas) or the end of the list. A clip needs no
    /// draw of its own: each quad is cut to the clip before it is drawn,
    /// on whole pixels, so nothing outside it is touched.
    pub fn play(&mut self, gpu: &Gpu, ops: &[Op], store: &Store, dropped: &[u32]) -> DrawStats {
        let gl = &gpu.gl;
        self.stats = DrawStats::default();
        for id in dropped {
            // A mask's room in the atlas comes back when the atlas is next
            // cleared.
            self.atlas.at.remove(id);
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
            gl.disable(glow::SCISSOR_TEST);
        }
        let all = Rect::new(0, 0, self.logical.0, self.logical.1);
        self.clip = all;
        for op in ops {
            match *op {
                Op::Clip(r) => self.clip = r.intersect(all),
                Op::Fill(r, c) => self.push_shape(gl, r, 0, None, 0, c),
                Op::FillRound { r, radius, c } => {
                    let rad = radius.clamp(0, r.w.min(r.h) / 2);
                    self.push_shape(gl, r, rad, None, 0, c);
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
                    self.push_shape(gl, r, rad, inner, irad, c);
                }
                Op::Mask { x, y, id, c } => {
                    if let Some(m) = store.mask(id) {
                        let (w, h) = (m.w as i32, m.h as i32);
                        if w > 0 && h > 0 {
                            self.mask(gl, id, Rect::new(x, y, w, h), &m.data, c);
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

    /// One quad of the batch, cut to the clip: `area` is the part drawn.
    fn push_quad(&mut self, gl: &glow::Context, area: Rect, mut v: Vertex) {
        if self.verts.len() >= MAX_QUADS * 4 {
            self.flush(gl);
        }
        let (x0, y0) = (area.x as f32, area.y as f32);
        let (x1, y1) = (area.right() as f32, area.bottom() as f32);
        for pos in [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] {
            v.pos = pos;
            self.verts.push(v);
        }
    }

    fn push_shape(
        &mut self,
        gl: &glow::Context,
        r: Rect,
        rad: i32,
        inner: Option<Rect>,
        irad: i32,
        c: Color,
    ) {
        let area = r.intersect(self.clip);
        if area.w <= 0 || area.h <= 0 {
            return;
        }
        let v = Vertex {
            pos: [0.0; 2],
            rect: [r.x as f32, r.y as f32, r.w as f32, r.h as f32],
            inner: inner.map_or([0.0; 4], |i| {
                [i.x as f32, i.y as f32, i.w as f32, i.h as f32]
            }),
            rad: [rad as f32, irad as f32, 0.0],
            color: [c.r, c.g, c.b, c.a],
        };
        self.push_quad(gl, area, v);
    }

    /// A mask at `place`, from the atlas (uploaded there the first time),
    /// or alone in its own texture if it is bigger than the atlas.
    fn mask(&mut self, gl: &glow::Context, id: u32, place: Rect, data: &[u8], c: Color) {
        let area = place.intersect(self.clip);
        if area.w <= 0 || area.h <= 0 {
            return;
        }
        let at = match self.atlas.at.get(&id) {
            Some(&at) => Some(at),
            None => self.upload_to_atlas(gl, id, place.w, place.h, data),
        };
        let Some((ax, ay)) = at else {
            self.flush(gl);
            let tex = self.texture(gl, id, || {
                (place.w, place.h, glow::LUMINANCE, data.to_vec())
            });
            self.stats.masks_alone += 1;
            self.draw_quad(gl, tex, place, Some(c));
            return;
        };
        let v = Vertex {
            pos: [0.0; 2],
            rect: [
                place.x as f32,
                place.y as f32,
                place.w as f32,
                place.h as f32,
            ],
            inner: [ax as f32, ay as f32, 0.0, 0.0],
            rad: [0.0, 0.0, 1.0],
            color: [c.r, c.g, c.b, c.a],
        };
        self.push_quad(gl, area, v);
    }

    /// Puts a mask in the atlas, clearing it first if it is full (after
    /// drawing what is waiting, which may read what is cleared). None if
    /// the mask is bigger than the atlas.
    fn upload_to_atlas(
        &mut self,
        gl: &glow::Context,
        id: u32,
        w: i32,
        h: i32,
        data: &[u8],
    ) -> Option<(i32, i32)> {
        if w > self.atlas.size.0 || h > self.atlas.size.1 {
            return None;
        }
        let at = match self.atlas.place(w, h) {
            Some(at) => at,
            None => {
                self.flush(gl);
                self.atlas.clear();
                self.stats.atlas_cleared += 1;
                self.atlas.place(w, h)?
            }
        };
        // SAFETY: `data` holds w * h bytes, rows unpadded (UNPACK_ALIGNMENT
        // is 1), and (at, w, h) lies inside the atlas.
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas.tex));
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                at.0,
                at.1,
                w,
                h,
                glow::LUMINANCE,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(data)),
            );
        }
        self.atlas.at.insert(id, at);
        self.stats.uploads += 1;
        Some(at)
    }

    /// Draws the shapes and masks gathered since the last flush, in one
    /// call.
    fn flush(&mut self, gl: &glow::Context) {
        if self.verts.is_empty() {
            return;
        }
        let stride = std::mem::size_of::<Vertex>() as i32;
        // SAFETY: `verts` is plain data (repr(C) floats and bytes); the
        // attributes below match its layout; the index buffer covers
        // MAX_QUADS quads and `verts` holds at most that many.
        unsafe {
            gl.use_program(Some(self.batch.id));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas.tex));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let bytes = std::slice::from_raw_parts(
                self.verts.as_ptr() as *const u8,
                self.verts.len() * stride as usize,
            );
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
            let mut off = 0;
            for (i, &(_, n, kind, norm)) in VERTEX_ATTRS.iter().enumerate() {
                gl.enable_vertex_attrib_array(i as u32);
                gl.vertex_attrib_pointer_f32(i as u32, n, kind, norm, stride, off);
                off += n * if kind == glow::FLOAT { 4 } else { 1 };
            }
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.ibo));
            let quads = self.verts.len() / 4;
            gl.draw_elements(glow::TRIANGLES, quads as i32 * 6, glow::UNSIGNED_SHORT, 0);
            for i in 0..VERTEX_ATTRS.len() {
                gl.disable_vertex_attrib_array(i as u32);
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
            for (nx, p) in src.as_chunks::<4>().0.iter().enumerate() {
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

/// How far two frames of XRGB pixels are apart, channel by channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// The largest difference in any channel of any pixel.
    pub max: u32,
    /// Pixels with a channel more than 1 apart.
    pub over_1: usize,
    /// Pixels that differ at all.
    pub differ: usize,
}

/// Compares two frames of the same size (alpha bits are ignored).
pub fn compare(a: &[u32], b: &[u32]) -> Diff {
    assert_eq!(a.len(), b.len(), "frames of different sizes");
    let mut d = Diff::default();
    for (x, y) in a.iter().zip(b) {
        let m = [16, 8, 0]
            .iter()
            .map(|s| ((x >> s) & 0xFF).abs_diff((y >> s) & 0xFF))
            .max()
            .unwrap_or(0);
        d.max = d.max.max(m);
        d.differ += (m > 0) as usize;
        d.over_1 += (m > 1) as usize;
    }
    d
}

pub mod compute;

#[cfg(test)]
mod tests;
