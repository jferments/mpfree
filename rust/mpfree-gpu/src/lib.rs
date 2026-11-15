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
//! A frame is drawn into a [`Target`]: the renderer's own texture, which
//! keeps its pixels between frames as the software canvas does, or a
//! buffer the panel shows ([`scanout`]), so a frame's list only redraws
//! what changed. Native row 0 (the panel's top row) is GL's window row 0,
//! which is the first row of the buffer in memory: what the display
//! controller scans out first. Only the part of the frame a list changes is
//! drawn (a scissor around it), so the GPU leaves the rest alone.
//!
//! Most of a screen is plain colour: backgrounds, panels, the straight
//! parts of rounded boxes and outlines. Those pixels take the cheapest path
//! (the colour, nothing worked out); only the corners of rounded shapes
//! work out how much of each pixel they cover, and an outline is drawn as
//! its four edges and four corners, never as a whole box with the middle
//! thrown away. [`DrawStats`] counts both kinds of pixel.
//!
//! [`Gpu::surfaceless`] opens EGL without any display (Mesa's surfaceless
//! platform), as the MPC's offscreen GPU tests did on 9 Nov; it reads
//! `libEGL.so.1` at run time, so nothing links against the system's GPU
//! libraries. [`Renderer::read_native`] reads the frame back, for the tests
//! and for the copy to `/dev/fb0` the panel bench still times.
//!
//! Vector shapes (curves, knob rings, graphs: [`mpfree_ui::Shape`]) arrive
//! as their parts, not as pixels. The GPU works out each part's coverage
//! itself into a second atlas (with `GL_EXT_blend_minmax`, so overlapping
//! parts take the larger coverage, as software does), once per shape, and
//! then draws the shape like a mask, in the same draw call as everything
//! else. Without that extension, or for a shape bigger than its atlas, it
//! takes software's coverage and draws it as a mask.
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
use mpfree_ui::{Color, Part, Rect, Shape};

/// An EGL display and an OpenGL ES 2 context, current on this thread.
pub struct Gpu {
    pub(crate) egl: egl::DynamicInstance<egl::EGL1_4>,
    pub(crate) display: egl::Display,
    context: egl::Context,
    /// EGL's version and vendor, as the display reports them.
    egl_version: String,
    /// The display's EGL extensions, space separated.
    egl_extensions: String,
    pub gl: glow::Context,
    /// Kept loaded while `gl` holds its functions.
    _gles: Option<libloading::Library>,
}

/// The `Gpu`s alive in this process (see `Gpu::surfaceless_es`).
static LIVE: std::sync::Mutex<usize> = std::sync::Mutex::new(0);

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
        // EGL gives every Gpu in the process this same display, and
        // eglTerminate ends it for all of them: count the live ones, under a
        // lock, so the last to go ends it and none ends it while another
        // is being set up.
        let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        let (context, egl_version, egl_extensions) = match Gpu::context(&egl, display, major, minor)
        {
            Ok(made) => made,
            Err(e) => {
                if *live == 0 {
                    let _ = egl.terminate(display);
                }
                return Err(e);
            }
        };
        *live += 1;
        drop(live);
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
            egl_extensions,
            gl,
            _gles: gles,
        })
    }

    /// Initialises `display` and makes an OpenGL ES `major`.`minor`
    /// context current on this thread; also EGL's version and vendor.
    fn context(
        egl: &egl::DynamicInstance<egl::EGL1_4>,
        display: egl::Display,
        major: i32,
        minor: i32,
    ) -> Result<(egl::Context, String, String), String> {
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
        if let Err(e) = egl.make_current(display, None, None, Some(context)) {
            let _ = egl.destroy_context(display, context);
            return Err(format!("eglMakeCurrent with no surface: {e}"));
        }
        Ok((context, egl_version, display_ext))
    }

    /// Whether the display has the EGL extension `name`.
    pub fn has_egl_extension(&self, name: &str) -> bool {
        self.egl_extensions.split(' ').any(|e| e == name)
    }

    /// Whether the context has the OpenGL ES extension `name`.
    pub fn has_gl_extension(&self, name: &str) -> bool {
        self.gl.supported_extensions().contains(name)
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
        let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        *live = live.saturating_sub(1);
        if *live == 0 {
            let _ = self.egl.terminate(self.display);
        }
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

    /// The native rectangle a logical one lands on (its edges map to edges).
    fn rect(&self, r: Rect) -> Rect {
        let at = |x: i32, y: i32| {
            let (x, y) = (x as f32, y as f32);
            (
                (self.m[0] * x + self.m[1] * y + self.t[0]).round() as i32,
                (self.m[2] * x + self.m[3] * y + self.t[1]).round() as i32,
            )
        };
        let (ax, ay) = at(r.x, r.y);
        let (bx, by) = at(r.right(), r.bottom());
        Rect::new(ax.min(bx), ay.min(by), (ax - bx).abs(), (ay - by).abs())
    }
}

// Native pixel (nx, ny) is GL's window pixel (nx, ny): row 0 is the panel's
// top row and the first row of the target in memory, which is what a
// display controller scans out first. So nothing is flipped: a vertex at
// native n goes to n / native * 2 - 1, and a fragment's native position is
// gl_FragCoord.xy.

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
    gl_Position = vec4(n / u_native * 2.0 - 1.0, 0.0, 1.0);
    v_rect = a_rect;
    v_inner = a_inner;
    v_rad = a_rad;
    v_color = a_color;
}
"#;

/// Plain quads, shapes and masks in one draw. A plain quad (`v_rad.z` 2)
/// is its colour and nothing else: the cheap path most pixels take. A
/// shape (`v_rad.z` 0) takes the software canvas's coverage
/// (mpfree_ui::canvas::coverage), per pixel centre in logical coordinates;
/// a stroke is the outer shape less the inner one. A mask (`v_rad.z` 1)
/// sits at `v_rect.xy` and reads its coverage from the atlas at
/// `v_inner.xy`, each pixel exactly its own texel whatever the turn; a
/// vector shape (`v_rad.z` 3) does the same from the shapes' atlas, where
/// the GPU worked it out (`RASTER_FS`). `u_m` is a turn, so its inverse is
/// its transpose.
const BATCH_FS: &str = r#"#version 100
precision highp float;
uniform vec2 u_native;
uniform mat2 u_m;
uniform vec2 u_t;
uniform vec2 u_atlas_size;
uniform sampler2D u_atlas;
uniform vec2 u_shapes_size;
uniform sampler2D u_shapes;
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
    if (v_rad.z > 1.5 && v_rad.z < 2.5) {
        gl_FragColor = v_color;
        return;
    }
    vec2 f = (gl_FragCoord.xy - u_t) * u_m;
    float cov;
    if (v_rad.z > 2.5) {
        vec2 texel = floor(f - v_rect.xy) + 0.5 + v_inner.xy;
        cov = texture2D(u_shapes, texel / u_shapes_size).r;
    } else if (v_rad.z > 0.5) {
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
    gl_Position = vec4(n / u_native * 2.0 - 1.0, 0.0, 1.0);
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
    vec2 f = (gl_FragCoord.xy - u_t) * u_m;
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

/// Native rectangles copied from another frame of the same size, pixel for
/// pixel (no turn: both are already in the panel's order).
const COPY_VS: &str = r#"#version 100
uniform vec2 u_native;
attribute vec2 a_pos;
void main() {
    gl_Position = vec4(a_pos / u_native * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const COPY_FS: &str = r#"#version 100
precision highp float;
uniform vec2 u_native;
uniform sampler2D u_tex;
void main() {
    gl_FragColor = texture2D(u_tex, gl_FragCoord.xy / u_native);
}
"#;

/// A vector shape's parts, each a quad over the pixels it may cover, drawn
/// into its place in the shapes' atlas (window pixels are atlas texels, row
/// 0 the texture's first row). `a_s.xy` turns a texel into the logical
/// pixel it stands for; `a_s.z` is the part's kind.
const RASTER_VS: &str = r#"#version 100
uniform vec2 u_size;
attribute vec2 a_pos;
attribute vec4 a_p;
attribute vec4 a_q;
attribute vec4 a_r;
attribute vec4 a_s;
varying vec4 v_p;
varying vec4 v_q;
varying vec4 v_r;
varying vec4 v_s;
void main() {
    gl_Position = vec4(a_pos / u_size * 2.0 - 1.0, 0.0, 1.0);
    v_p = a_p;
    v_q = a_q;
    v_r = a_r;
    v_s = a_s;
}
"#;

/// Each part's coverage at the logical pixel's centre, worked out as the
/// software rasteriser does (mpfree_ui::shape): a segment (kind 0: ends in
/// `p`, half its width in `q.x`), a disc (1: centre and radius in `p`), an
/// arc (2: centre, radius and half width in `p`, its angles in `q`, its
/// ends in `r`) and the area under a line (3: the line's ends in `p`, the
/// base and coverage in `q`). Drawn with MAX blending, so where parts
/// overlap the larger coverage stays.
const RASTER_FS: &str = r#"#version 100
precision highp float;
varying vec4 v_p;
varying vec4 v_q;
varying vec4 v_r;
varying vec4 v_s;
float round_away(float v) {
    return sign(v) * floor(abs(v) + 0.5);
}
void main() {
    vec2 f = gl_FragCoord.xy + v_s.xy;
    float cov = 0.0;
    if (v_s.z < 0.5) {
        vec2 a = v_p.xy;
        vec2 d = v_p.zw - a;
        float len2 = dot(d, d);
        float t = len2 > 0.0 ? clamp(dot(f - a, d) / len2, 0.0, 1.0) : 0.0;
        cov = v_q.x + 0.5 - length(a + t * d - f);
    } else if (v_s.z < 1.5) {
        cov = v_p.z + 0.5 - length(f - v_p.xy);
    } else if (v_s.z < 2.5) {
        vec2 dd = f - v_p.xy;
        float ang = mod(atan(dd.y, dd.x) - v_q.x, 6.2831853) + v_q.x;
        float d;
        if (ang <= v_q.y) {
            d = abs(length(dd) - v_p.z);
        } else {
            d = min(length(f - v_r.xy), length(f - v_r.zw));
        }
        cov = v_p.w + 0.5 - d;
    } else {
        vec2 a = v_p.xy;
        vec2 b = v_p.zw;
        if (f.x < a.x || f.x >= b.x) discard;
        float yc = a.y + (b.y - a.y) * (f.x - a.x) / (b.x - a.x);
        float row = floor(f.y);
        if (row >= round_away(min(yc, v_q.x)) && row < round_away(max(yc, v_q.x))) {
            cov = v_q.y;
        }
    }
    if (cov <= 0.0) discard;
    gl_FragColor = vec4(min(cov, 1.0), 0.0, 0.0, 1.0);
}
"#;

/// The batch's kinds of quad (`Vertex::rad[2]`).
const SHADED: f32 = 0.0;
const MASK: f32 = 1.0;
const PLAIN: f32 = 2.0;
const SHAPE: f32 = 3.0;

/// One corner of a part's quad (see `RASTER_VS`): 72 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
struct PartVertex {
    pos: [f32; 2],
    p: [f32; 4],
    q: [f32; 4],
    r: [f32; 4],
    s: [f32; 4],
}

const PART_ATTRS: [(&str, i32); 5] = [("a_pos", 2), ("a_p", 4), ("a_q", 4), ("a_r", 4), ("a_s", 4)];

/// One corner of a quad in the batch (see `BATCH_VS`): 56 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 2],
    /// A shape's rectangle, or where a mask sits (x, y, w, h).
    rect: [f32; 4],
    /// A stroke's inner rectangle, or the mask's place in the atlas (x, y).
    inner: [f32; 4],
    /// The radii (outer, inner) and the kind: SHADED, MASK or PLAIN.
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

/// The shapes' atlas, at most: 4 MiB of RGBA texels (the GPU draws into
/// it, so it can't be one byte a texel as the masks' is), about eight
/// times the shape pixels of the busiest sample page.
const SHAPE_ATLAS_SIZE: (i32, i32) = (1024, 1024);

/// A frame's changed rectangles are merged down to at most this many.
const MAX_DAMAGE: usize = 16;

/// Two changed rectangles become one when the rectangle holding both is at
/// most this many pixels bigger than the two (a 32 x 32 square).
const DAMAGE_SLACK: i64 = 1024;

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
    /// one more, and so is the copy that brings a panel buffer up to date.
    pub draws: usize,
    /// Masks and images uploaded this frame.
    pub uploads: usize,
    /// Times the atlas was full and was cleared, so that the masks still in
    /// use were uploaded again (a cost, not a fault; counted to be seen).
    pub atlas_cleared: usize,
    /// Masks too big for the atlas, drawn alone in their own textures.
    pub masks_alone: usize,
    /// Vector shapes the GPU worked out into the shapes' atlas this frame
    /// (each once, until the atlas is cleared or the store drops it).
    pub shapes_drawn: usize,
    /// Times the shapes' atlas was full and was cleared.
    pub shapes_cleared: usize,
    /// Pixels drawn plain: the colour and nothing else (the cheap path).
    pub plain_px: usize,
    /// Pixels that work something out or read a texture: rounded corners,
    /// outlines too small to split, text and other masks, images.
    pub shaded_px: usize,
    /// Pixels copied from the frame shown last, to bring a panel buffer up
    /// to date before the list is drawn into it.
    pub copied_px: usize,
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
    /// so text of about the same size shares shelves. When no shelf of
    /// about its height has room and there is no room left for a new
    /// shelf, it goes on the lowest taller shelf that has room, before the
    /// atlas counts as full (a small knob beside a big graph's layers).
    fn place(&mut self, w: i32, h: i32) -> Option<(i32, i32)> {
        if w > self.size.0 {
            return None;
        }
        let step = (h + 7) & !7;
        // The lowest shelf with room, of about this height when `snug`.
        let lowest = |shelves: &[(i32, i32, i32)], snug: bool| {
            shelves
                .iter()
                .enumerate()
                .filter(|&(_, &(_, sh, x))| {
                    sh >= h && (!snug || sh <= step + 8) && x + w <= self.size.0
                })
                .min_by_key(|&(_, &(_, sh, _))| sh)
                .map(|(i, _)| i)
        };
        let i = match lowest(&self.shelves, true) {
            Some(i) => i,
            None => {
                let top = self.shelves.last().map_or(0, |&(y, sh, _)| y + sh);
                let sh = step.min(self.size.1 - top);
                if sh >= h {
                    self.shelves.push((top, sh, 0));
                    self.shelves.len() - 1
                } else {
                    lowest(&self.shelves, false)?
                }
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

/// A framebuffer a list can be drawn into, at the panel's native size: the
/// renderer's own frame, or a buffer the panel shows ([`scanout`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub fbo: glow::Framebuffer,
    pub texture: glow::Texture,
}

impl Target {
    /// A target that draws into `texture` (already sized and filled, or
    /// backed by a buffer from elsewhere).
    pub fn for_texture(gpu: &Gpu, texture: glow::Texture) -> Result<Target, String> {
        let gl = &gpu.gl;
        // SAFETY: plain GL calls on the current context.
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32);
            }
            for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32);
            }
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(fbo);
                return Err(format!(
                    "the frame's framebuffer is incomplete ({status:#x})"
                ));
            }
            Ok(Target { fbo, texture })
        }
    }

    /// A new black frame of `w` x `h` in a texture of its own.
    pub fn new(gpu: &Gpu, w: i32, h: i32) -> Result<Target, String> {
        let gl = &gpu.gl;
        // SAFETY: plain GL calls on the current context.
        let texture = unsafe {
            let t = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                w,
                h,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            t
        };
        let target = Target::for_texture(gpu, texture).inspect_err(|_| {
            // SAFETY: the texture made above.
            unsafe { gl.delete_texture(texture) };
        })?;
        target.clear(gpu, w, h);
        Ok(target)
    }

    /// Paints the whole target black.
    pub fn clear(&self, gpu: &Gpu, w: i32, h: i32) {
        let gl = &gpu.gl;
        // SAFETY: plain GL calls on the current context.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.disable(glow::SCISSOR_TEST);
            gl.viewport(0, 0, w, h);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    /// Frees the framebuffer, and the texture too when `with_texture`.
    pub fn delete(self, gpu: &Gpu, with_texture: bool) {
        // SAFETY: objects this target holds.
        unsafe {
            gpu.gl.delete_framebuffer(self.fbo);
            if with_texture {
                gpu.gl.delete_texture(self.texture);
            }
        }
    }

    /// The target as the panel takes it: native rows top to bottom, one
    /// XRGB8888 word per pixel. Waits for the GPU to finish.
    pub fn read(&self, gpu: &Gpu, w: i32, h: i32) -> Vec<u32> {
        let gl = &gpu.gl;
        let (w, h) = (w as usize, h as usize);
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
        // Window row 0 is native row 0, so the rows come in the panel's order.
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32)
            .collect()
    }
}

/// The rectangles a frame changes, merged so there are few: one that holds
/// another swallows it, two whose union wastes little become one, and past
/// [`MAX_DAMAGE`] they all become one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    rects: Vec<Rect>,
}

fn area(r: Rect) -> i64 {
    if r.is_empty() {
        0
    } else {
        r.w as i64 * r.h as i64
    }
}

impl Damage {
    pub fn add(&mut self, r: Rect) {
        if r.is_empty() {
            return;
        }
        let mut r = r;
        let mut i = 0;
        while i < self.rects.len() {
            let e = self.rects[i];
            let u = e.union(r);
            if area(u) <= area(e) + area(r) + DAMAGE_SLACK {
                r = u;
                self.rects.swap_remove(i);
                i = 0;
            } else {
                i += 1;
            }
        }
        self.rects.push(r);
        if self.rects.len() > MAX_DAMAGE {
            let all = self.bounds();
            self.rects = vec![all];
        }
    }

    pub fn extend(&mut self, rects: &[Rect]) {
        for r in rects {
            self.add(*r);
        }
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }

    /// The rectangle holding them all.
    pub fn bounds(&self) -> Rect {
        self.rects.iter().fold(Rect::default(), |a, r| a.union(*r))
    }

    /// The pixels they cover (overlaps counted twice).
    pub fn pixels(&self) -> usize {
        self.rects.iter().map(|r| area(*r) as usize).sum()
    }
}

/// The logical rectangles a list draws into, each cut to its clip, merged.
pub fn list_damage(ops: &[Op], store: &Store, w: i32, h: i32) -> Damage {
    let all = Rect::new(0, 0, w, h);
    let mut clip = all;
    let mut d = Damage::default();
    for op in ops {
        let area = match *op {
            Op::Clip(r) => {
                clip = r.intersect(all);
                continue;
            }
            Op::Fill(r, _) | Op::FillRound { r, .. } | Op::StrokeRound { r, .. } => r,
            Op::Mask { x, y, id, .. } => match store.mask(id) {
                Some(m) => Rect::new(x, y, m.w as i32, m.h as i32),
                None => continue,
            },
            Op::Image { x, y, id } => match store.image(id) {
                Some(img) => Rect::new(x, y, img.w, img.h),
                None => continue,
            },
            Op::Shape { id, .. } => match store.shape(id) {
                Some(sh) => sh.r,
                None => continue,
            },
        };
        d.add(area.intersect(clip));
    }
    d
}

/// The GPU's copy of the frame, in the panel's orientation, and what it
/// needs to play lists into it or into a buffer the panel shows.
pub struct Renderer {
    native: (i32, i32),
    logical: (i32, i32),
    turn: Turn,
    /// The renderer's own frame, which [`Renderer::play`] draws into.
    frame: Target,
    batch: Program,
    quad: Program,
    copy: Program,
    place: Option<glow::UniformLocation>,
    color: Option<glow::UniformLocation>,
    image: Option<glow::UniformLocation>,
    vbo: glow::Buffer,
    /// The corners' order, two triangles per quad, for every batch.
    ibo: glow::Buffer,
    verts: Vec<Vertex>,
    atlas: Atlas,
    /// Where the GPU works out vector shapes: an RGBA texture it draws
    /// into, and the program that does it.
    shapes: Atlas,
    shapes_fbo: Target,
    raster: Program,
    part_verts: Vec<PartVertex>,
    /// Whether MAX blending is there (`GL_EXT_blend_minmax`); without it,
    /// shapes take software's coverage.
    minmax: bool,
    /// What `play_on` draws into, and its scissor, to go back to after
    /// working out a shape.
    target_fbo: glow::Framebuffer,
    scissor: Rect,
    /// Images and masks too big for the atlas, one texture each.
    textures: HashMap<u32, (glow::Texture, i32, i32)>,
    clip: Rect,
    stats: DrawStats,
    /// The native rectangles the last list drew into.
    damage: Vec<Rect>,
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

/// The pixels (first and last column, first and last row, logical, cut
/// to `r`) whose centres a part may cover: what the software rasteriser
/// looks at (mpfree_ui::shape). None if it covers none of `r`.
fn part_pixels(r: Rect, part: &Part) -> Option<(i32, i32, i32, i32)> {
    let around = |x0: f32, y0: f32, x1: f32, y1: f32, pad: f32| {
        (
            ((x0.min(x1) - pad).floor() as i32).max(r.x),
            ((y0.min(y1) - pad).floor() as i32).max(r.y),
            ((x0.max(x1) + pad).ceil() as i32).min(r.right() - 1),
            ((y0.max(y1) + pad).ceil() as i32).min(r.bottom() - 1),
        )
    };
    let (xa, ya, xb, yb) = match *part {
        Part::Segment { a, b, width } => around(a.0, a.1, b.0, b.1, width * 0.5 + 1.0),
        Part::Disc { c, radius } => around(c.0, c.1, c.0, c.1, radius + 1.0),
        Part::Arc {
            c,
            radius,
            a0,
            a1,
            width,
        } => {
            if a1 <= a0 {
                return None;
            }
            around(c.0, c.1, c.0, c.1, radius + width * 0.5 + 1.0)
        }
        Part::Under { a, b, base, .. } => {
            if b.0 <= a.0 {
                return None;
            }
            let lo = a.1.min(b.1).min(base).round() - 1.0;
            let hi = a.1.max(b.1).max(base).round() + 1.0;
            (
                (a.0.floor() as i32).max(r.x),
                (lo as i32).max(r.y),
                (b.0.ceil() as i32 - 1).min(r.right() - 1),
                (hi as i32).min(r.bottom() - 1),
            )
        }
    };
    (xa <= xb && ya <= yb).then_some((xa, ya, xb, yb))
}

/// A part's numbers for `RASTER_FS`: `p`, `q`, `r` and its kind.
fn part_numbers(part: &Part) -> ([f32; 4], [f32; 4], [f32; 4], f32) {
    match *part {
        Part::Segment { a, b, width } => (
            [a.0, a.1, b.0, b.1],
            [width * 0.5, 0.0, 0.0, 0.0],
            [0.0; 4],
            0.0,
        ),
        Part::Disc { c, radius } => ([c.0, c.1, radius, 0.0], [0.0; 4], [0.0; 4], 1.0),
        Part::Arc {
            c,
            radius,
            a0,
            a1,
            width,
        } => {
            // The ends as software works them out, in f32 on the CPU.
            let e0 = (c.0 + radius * a0.cos(), c.1 + radius * a0.sin());
            let e1 = (c.0 + radius * a1.cos(), c.1 + radius * a1.sin());
            (
                [c.0, c.1, radius, width * 0.5],
                [a0, a1, 0.0, 0.0],
                [e0.0, e0.1, e1.0, e1.1],
                2.0,
            )
        }
        Part::Under { a, b, base, alpha } => {
            ([a.0, a.1, b.0, b.1], [base, alpha, 0.0, 0.0], [0.0; 4], 3.0)
        }
    }
}

/// The four `k` x `k` corners of `r`.
fn corners(r: Rect, k: i32) -> [Rect; 4] {
    [
        Rect::new(r.x, r.y, k, k),
        Rect::new(r.right() - k, r.y, k, k),
        Rect::new(r.x, r.bottom() - k, k, k),
        Rect::new(r.right() - k, r.bottom() - k, k, k),
    ]
}

impl Renderer {
    /// A black frame for a panel of `nw` x `nh` native pixels mounted
    /// turned by `rotation`.
    pub fn new(gpu: &Gpu, nw: i32, nh: i32, rotation: Rotation) -> Result<Renderer, String> {
        Renderer::with_atlases(gpu, nw, nh, rotation, ATLAS_SIZE, SHAPE_ATLAS_SIZE)
    }

    /// As `new`, with an atlas of at most `atlas` texels (the tests make a
    /// small one to fill it).
    #[cfg(test)]
    fn with_atlas(
        gpu: &Gpu,
        nw: i32,
        nh: i32,
        rotation: Rotation,
        atlas: (i32, i32),
    ) -> Result<Renderer, String> {
        Renderer::with_atlases(gpu, nw, nh, rotation, atlas, SHAPE_ATLAS_SIZE)
    }

    /// As `new`, with atlases of at most `atlas` and `shapes` texels.
    fn with_atlases(
        gpu: &Gpu,
        nw: i32,
        nh: i32,
        rotation: Rotation,
        atlas: (i32, i32),
        shapes: (i32, i32),
    ) -> Result<Renderer, String> {
        let gl = &gpu.gl;
        let logical = match rotation {
            Rotation::Deg0 | Rotation::Deg180 => (nw, nh),
            Rotation::Deg90 | Rotation::Deg270 => (nh, nw),
        };
        let names: Vec<&str> = VERTEX_ATTRS.iter().map(|a| a.0).collect();
        let batch = compile(gl, BATCH_VS, BATCH_FS, &names)?;
        let quad = compile(gl, QUAD_VS, QUAD_FS, &["a_pos"])?;
        let copy = compile(gl, COPY_VS, COPY_FS, &["a_pos"])?;
        let part_names: Vec<&str> = PART_ATTRS.iter().map(|a| a.0).collect();
        let raster = compile(gl, RASTER_VS, RASTER_FS, &part_names)?;
        let frame = Target::new(gpu, nw, nh)?;
        let minmax = gpu.has_gl_extension("GL_EXT_blend_minmax");
        // SAFETY: plain GL calls on the current context.
        unsafe {
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
            // The shapes' atlas: RGBA, which the GPU can draw into.
            let shapes_size = (shapes.0.min(max), shapes.1.min(max));
            let stex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(stex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                shapes_size.0,
                shapes_size.1,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            let shapes_fbo = Target::for_texture(gpu, stex)?;
            // Clean from the start (see `clear_shapes`).
            gl.disable(glow::SCISSOR_TEST);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            let raster_size = gl.get_uniform_location(raster.id, "u_size");
            gl.use_program(Some(raster.id));
            gl.uniform_2_f32(
                raster_size.as_ref(),
                shapes_size.0 as f32,
                shapes_size.1 as f32,
            );
            let place = gl.get_uniform_location(quad.id, "u_place");
            let color = gl.get_uniform_location(quad.id, "u_color");
            let image = gl.get_uniform_location(quad.id, "u_image");
            let r = Renderer {
                native: (nw, nh),
                logical,
                turn: Turn::new(rotation, nw as f32, nh as f32),
                frame,
                batch,
                quad,
                copy,
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
                shapes: Atlas {
                    tex: stex,
                    size: shapes_size,
                    shelves: Vec::new(),
                    at: HashMap::new(),
                },
                shapes_fbo,
                raster,
                part_verts: Vec::new(),
                minmax,
                target_fbo: frame.fbo,
                scissor: Rect::new(0, 0, nw, nh),
                textures: HashMap::new(),
                clip: Rect::new(0, 0, logical.0, logical.1),
                stats: DrawStats::default(),
                damage: Vec::new(),
            };
            for p in [&r.batch, &r.quad, &r.copy] {
                r.set_common(gl, p);
            }
            gl.use_program(Some(r.batch.id));
            gl.uniform_2_f32(
                gl.get_uniform_location(r.batch.id, "u_atlas_size").as_ref(),
                size.0 as f32,
                size.1 as f32,
            );
            gl.uniform_2_f32(
                gl.get_uniform_location(r.batch.id, "u_shapes_size")
                    .as_ref(),
                shapes_size.0 as f32,
                shapes_size.1 as f32,
            );
            gl.uniform_1_i32(gl.get_uniform_location(r.batch.id, "u_atlas").as_ref(), 0);
            gl.uniform_1_i32(gl.get_uniform_location(r.batch.id, "u_shapes").as_ref(), 1);
            Ok(r)
        }
    }

    /// The logical (landscape) size the screens draw at.
    pub fn logical_size(&self) -> (i32, i32) {
        self.logical
    }

    /// The panel's native size, which every target has.
    pub fn native_size(&self) -> (i32, i32) {
        self.native
    }

    /// The renderer's own frame, in the panel's orientation.
    pub fn frame(&self) -> Target {
        self.frame
    }

    /// The texture that holds the renderer's own frame.
    pub fn frame_texture(&self) -> glow::Texture {
        self.frame.texture
    }

    /// The native rectangles the last list drew into (merged; see
    /// [`Damage`]).
    pub fn damage(&self) -> &[Rect] {
        &self.damage
    }

    /// The native rectangle a logical one lands on.
    pub fn to_native(&self, r: Rect) -> Rect {
        self.turn.rect(r)
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

    /// Forgets every mask and image it has uploaded. Call it before playing
    /// lists from a new store: a list recorded apart numbers its masks
    /// from 0 again, and without this the renderer would draw the masks it
    /// kept under those numbers (live test 3: the panel bench showed one
    /// page's glyphs and knob rings in the next page's text).
    pub fn forget_all(&mut self, gpu: &Gpu) {
        self.atlas.clear();
        self.part_verts.clear();
        self.clear_shapes(&gpu.gl);
        for (_, (t, _, _)) in self.textures.drain() {
            // SAFETY: a texture this renderer made.
            unsafe { gpu.gl.delete_texture(t) };
        }
    }

    /// Frees the masks and images the store let go (`dropped`, from
    /// `Store::take_changes`).
    pub fn drop_items(&mut self, gpu: &Gpu, dropped: &[u32]) {
        for id in dropped {
            // A mask's (or shape's) room in its atlas comes back when the
            // atlas is next cleared.
            self.atlas.at.remove(id);
            self.shapes.at.remove(id);
            if let Some((t, _, _)) = self.textures.remove(id) {
                // SAFETY: a texture this renderer made.
                unsafe { gpu.gl.delete_texture(t) };
            }
        }
    }

    /// Plays a frame's list into the renderer's own frame. Masks and images
    /// are uploaded the first time a list names them; `dropped` (from
    /// `Store::take_changes`) frees the ones the store let go. Every list
    /// played must come from the same store; for another store, call
    /// [`Renderer::forget_all`] first.
    ///
    /// Shapes and masks gather into one draw call until an image (or a
    /// mask too big for the atlas) or the end of the list. A clip needs no
    /// draw of its own: each quad is cut to the clip before it is drawn,
    /// on whole pixels, so nothing outside it is touched.
    pub fn play(&mut self, gpu: &Gpu, ops: &[Op], store: &Store, dropped: &[u32]) -> DrawStats {
        let frame = self.frame;
        self.play_on(gpu, frame, None, ops, store, dropped)
    }

    /// Plays a frame's list into `target`, as [`Renderer::play`] does into
    /// the renderer's own frame. With `refresh`, it first copies those
    /// native rectangles from `refresh`'s texture (a frame of the same
    /// size): what a panel buffer missed while another one was shown. Only
    /// the part of the target that the copy and the list touch is drawn
    /// (a scissor around both), so a small change costs the GPU little.
    pub fn play_on(
        &mut self,
        gpu: &Gpu,
        target: Target,
        refresh: Option<(glow::Texture, &[Rect])>,
        ops: &[Op],
        store: &Store,
        dropped: &[u32],
    ) -> DrawStats {
        let gl = &gpu.gl;
        self.stats = DrawStats::default();
        self.drop_items(gpu, dropped);
        let changed = list_damage(ops, store, self.logical.0, self.logical.1);
        self.damage = changed.rects().iter().map(|r| self.turn.rect(*r)).collect();
        let native = Rect::new(0, 0, self.native.0, self.native.1);
        let copies = refresh.map_or(&[][..], |(_, rects)| rects);
        let touched = copies
            .iter()
            .chain(&self.damage)
            .fold(Rect::default(), |a, r| a.union(*r))
            .intersect(native);
        if touched.is_empty() {
            return self.stats;
        }
        self.target_fbo = target.fbo;
        self.scissor = touched;
        // SAFETY: plain GL calls on the current context.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.fbo));
            gl.viewport(0, 0, self.native.0, self.native.1);
            // Window coordinates are native ones (row 0 at the top).
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(touched.x, touched.y, touched.w, touched.h);
        }
        if let Some((src, rects)) = refresh {
            self.copy_rects(gl, src, rects);
        }
        // SAFETY: plain GL calls on the current context.
        unsafe {
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        }
        let all = Rect::new(0, 0, self.logical.0, self.logical.1);
        self.clip = all;
        for op in ops {
            match *op {
                Op::Clip(r) => self.clip = r.intersect(all),
                Op::Fill(r, c) => self.plain(gl, r, c),
                Op::FillRound { r, radius, c } => {
                    let rad = radius.clamp(0, (r.w.min(r.h) / 2).max(0));
                    self.fill_round(gl, r, rad, c);
                }
                Op::StrokeRound {
                    r,
                    radius,
                    width,
                    c,
                } => {
                    let rad = radius.clamp(0, (r.w.min(r.h) / 2).max(0));
                    self.stroke_round(gl, r, rad, width.max(1), c);
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
                Op::Shape { id, c } => {
                    if let Some(sh) = store.shape(id) {
                        self.shape(gl, id, sh, c);
                    }
                }
            }
        }
        self.flush(gl);
        // SAFETY: a plain GL call.
        unsafe { gl.disable(glow::SCISSOR_TEST) };
        self.stats
    }

    /// Copies native rectangles from `src` into the bound target, pixel for
    /// pixel, in one draw call.
    fn copy_rects(&mut self, gl: &glow::Context, src: glow::Texture, rects: &[Rect]) {
        let native = Rect::new(0, 0, self.native.0, self.native.1);
        let mut v: Vec<f32> = Vec::with_capacity(rects.len() * 12);
        for r in rects {
            let r = r.intersect(native);
            if r.is_empty() {
                continue;
            }
            self.stats.copied_px += area(r) as usize;
            let (x0, y0) = (r.x as f32, r.y as f32);
            let (x1, y1) = (r.right() as f32, r.bottom() as f32);
            v.extend_from_slice(&[x0, y0, x1, y0, x1, y1, x0, y0, x1, y1, x0, y1]);
        }
        if v.is_empty() {
            return;
        }
        // SAFETY: plain GL calls with a vertex array of v.len() floats.
        unsafe {
            gl.disable(glow::BLEND);
            gl.use_program(Some(self.copy.id));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(src));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.draw_arrays(glow::TRIANGLES, 0, (v.len() / 2) as i32);
            gl.disable_vertex_attrib_array(0);
        }
        self.stats.draws += 1;
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

    /// `r`, cut to the clip, all in colour `c`: the cheap path.
    fn plain(&mut self, gl: &glow::Context, r: Rect, c: Color) {
        let area = r.intersect(self.clip);
        if area.is_empty() {
            return;
        }
        self.stats.plain_px += area.w as usize * area.h as usize;
        let v = Vertex {
            pos: [0.0; 2],
            rect: [0.0; 4],
            inner: [0.0; 4],
            rad: [0.0, 0.0, PLAIN],
            color: [c.r, c.g, c.b, c.a],
        };
        self.push_quad(gl, area, v);
    }

    /// The part `part` of the rounded shape `r` (less `inner`, for an
    /// outline), whose pixels work out how much of them it covers.
    #[allow(clippy::too_many_arguments)]
    fn shaded(
        &mut self,
        gl: &glow::Context,
        part: Rect,
        r: Rect,
        rad: i32,
        inner: Option<Rect>,
        irad: i32,
        c: Color,
    ) {
        let area = part.intersect(r).intersect(self.clip);
        if area.is_empty() {
            return;
        }
        self.stats.shaded_px += area.w as usize * area.h as usize;
        let v = Vertex {
            pos: [0.0; 2],
            rect: [r.x as f32, r.y as f32, r.w as f32, r.h as f32],
            inner: inner.map_or([0.0; 4], |i| {
                [i.x as f32, i.y as f32, i.w as f32, i.h as f32]
            }),
            rad: [rad as f32, irad as f32, SHADED],
            color: [c.r, c.g, c.b, c.a],
        };
        self.push_quad(gl, area, v);
    }

    /// A filled rounded rectangle: the corners work out their coverage,
    /// the rest (three bands) is plain, as the software canvas draws it.
    fn fill_round(&mut self, gl: &glow::Context, r: Rect, rad: i32, c: Color) {
        if r.is_empty() {
            return;
        }
        if rad == 0 {
            return self.plain(gl, r, c);
        }
        self.plain(gl, Rect::new(r.x, r.y + rad, r.w, r.h - 2 * rad), c);
        self.plain(gl, Rect::new(r.x + rad, r.y, r.w - 2 * rad, rad), c);
        self.plain(
            gl,
            Rect::new(r.x + rad, r.bottom() - rad, r.w - 2 * rad, rad),
            c,
        );
        for k in corners(r, rad) {
            self.shaded(gl, k, r, rad, None, 0, c);
        }
    }

    /// An outline: four plain edges between four corners that work out
    /// their coverage (outer shape less inner), as the software canvas
    /// draws it. The middle is never touched. An outline too small to split
    /// (its corners would meet, or it has no inside) is one shaded quad.
    fn stroke_round(&mut self, gl: &glow::Context, r: Rect, rad: i32, width: i32, c: Color) {
        if r.is_empty() {
            return;
        }
        let inner = r.inset(width);
        let irad = (rad - width).max(0);
        let m = rad.max(width);
        if inner.is_empty() {
            return self.shaded(gl, r, r, rad, None, 0, c);
        }
        if 2 * m > r.w || 2 * m > r.h {
            return self.shaded(gl, r, r, rad, Some(inner), irad, c);
        }
        self.plain(gl, Rect::new(r.x + m, r.y, r.w - 2 * m, width), c);
        self.plain(
            gl,
            Rect::new(r.x + m, r.bottom() - width, r.w - 2 * m, width),
            c,
        );
        self.plain(gl, Rect::new(r.x, r.y + m, width, r.h - 2 * m), c);
        self.plain(
            gl,
            Rect::new(r.right() - width, r.y + m, width, r.h - 2 * m),
            c,
        );
        for k in corners(r, m) {
            self.shaded(gl, k, r, rad, Some(inner), irad, c);
        }
    }

    /// A mask at `place`, from the atlas (uploaded there the first time),
    /// or alone in its own texture if it is bigger than the atlas.
    fn mask(&mut self, gl: &glow::Context, id: u32, place: Rect, data: &[u8], c: Color) {
        let area = place.intersect(self.clip);
        if area.is_empty() {
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
        self.stats.shaded_px += area.w as usize * area.h as usize;
        let v = Vertex {
            pos: [0.0; 2],
            rect: [
                place.x as f32,
                place.y as f32,
                place.w as f32,
                place.h as f32,
            ],
            inner: [ax as f32, ay as f32, 0.0, 0.0],
            rad: [0.0, 0.0, MASK],
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

    /// A vector shape, from the shapes' atlas (worked out there by the GPU
    /// the first time). Without MAX blending, or if it is bigger than that
    /// atlas, software's coverage drawn as a mask.
    fn shape(&mut self, gl: &glow::Context, id: u32, shape: &Shape, c: Color) {
        let place = shape.r;
        let area = place.intersect(self.clip);
        if area.is_empty() {
            return;
        }
        let at = match self.shapes.at.get(&id) {
            Some(&at) => Some(at),
            None => self.shape_to_atlas(gl, id, shape),
        };
        let Some((ax, ay)) = at else {
            // `mask` reads the coverage only to upload it: skip working it
            // out when the mask is up already.
            let up = self.atlas.at.contains_key(&id) || self.textures.contains_key(&id);
            let cov = if up { Vec::new() } else { shape.coverage() };
            self.mask(gl, id, place, &cov, c);
            return;
        };
        self.stats.shaded_px += area.w as usize * area.h as usize;
        let v = Vertex {
            pos: [0.0; 2],
            rect: [
                place.x as f32,
                place.y as f32,
                place.w as f32,
                place.h as f32,
            ],
            inner: [ax as f32, ay as f32, 0.0, 0.0],
            rad: [0.0, 0.0, SHAPE],
            color: [c.r, c.g, c.b, c.a],
        };
        self.push_quad(gl, area, v);
    }

    /// Gives a shape a free place in the shapes' atlas and queues its parts
    /// to be worked out there before the next batch is drawn (clearing the
    /// atlas first if it is full, after drawing what is waiting, which may
    /// read what is cleared). None without MAX blending or if the shape is
    /// bigger than the atlas.
    fn shape_to_atlas(&mut self, gl: &glow::Context, id: u32, shape: &Shape) -> Option<(i32, i32)> {
        let (w, h) = (shape.r.w, shape.r.h);
        if !self.minmax || w > self.shapes.size.0 || h > self.shapes.size.1 {
            return None;
        }
        let at = match self.shapes.place(w, h) {
            Some(at) => at,
            None => {
                self.flush(gl);
                self.clear_shapes(gl);
                // SAFETY: the context is current; this is mid-frame.
                unsafe { self.back_to_target(gl) };
                self.stats.shapes_cleared += 1;
                self.shapes.place(w, h)?
            }
        };
        let origin = (shape.r.x - at.0, shape.r.y - at.1);
        for part in &shape.parts {
            let Some((xa, ya, xb, yb)) = part_pixels(shape.r, part) else {
                continue;
            };
            let (p, q, r, kind) = part_numbers(part);
            let s = [origin.0 as f32, origin.1 as f32, kind, 0.0];
            let (x0, y0) = ((xa - origin.0) as f32, (ya - origin.1) as f32);
            let (x1, y1) = ((xb + 1 - origin.0) as f32, (yb + 1 - origin.1) as f32);
            for pos in [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] {
                self.part_verts.push(PartVertex { pos, p, q, r, s });
            }
        }
        self.shapes.at.insert(id, at);
        self.stats.shapes_drawn += 1;
        Some(at)
    }

    /// Empties the shapes' atlas: every texel back to 0, so each place it
    /// hands out is clean (places are not handed out twice until the next
    /// clear, and parts only ever raise coverage). Leaves the scissor off
    /// and the target bound; mid-frame, `back_to_target` follows.
    fn clear_shapes(&mut self, gl: &glow::Context) {
        self.shapes.clear();
        // SAFETY: plain GL calls on the current context.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.shapes_fbo.fbo));
            gl.disable(glow::SCISSOR_TEST);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.target_fbo));
        }
    }

    /// Back to drawing the frame `play_on` draws into, as it set things up.
    ///
    /// # Safety
    /// A GL context must be current.
    unsafe fn back_to_target(&self, gl: &glow::Context) {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.target_fbo));
        gl.viewport(0, 0, self.native.0, self.native.1);
        gl.enable(glow::SCISSOR_TEST);
        let t = self.scissor;
        gl.scissor(t.x, t.y, t.w, t.h);
    }

    /// Works out the queued shapes' parts in the shapes' atlas, all in one
    /// draw call (more only past MAX_QUADS parts).
    fn raster_shapes(&mut self, gl: &glow::Context) {
        if self.part_verts.is_empty() {
            return;
        }
        let stride = std::mem::size_of::<PartVertex>() as i32;
        // SAFETY: plain GL calls on the current context; `part_verts` is
        // plain data (repr(C) floats) laid out as the attributes say; each
        // draw covers at most MAX_QUADS quads, which the index buffer holds.
        unsafe {
            // Not bound for reading while drawn into (`flush` binds it
            // again), so no driver can see a feedback loop.
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.shapes_fbo.fbo));
            gl.viewport(0, 0, self.shapes.size.0, self.shapes.size.1);
            gl.disable(glow::SCISSOR_TEST);
            gl.blend_equation(glow::MAX);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.use_program(Some(self.raster.id));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.ibo));
            for chunk in self.part_verts.chunks(MAX_QUADS * 4) {
                let bytes = std::slice::from_raw_parts(
                    chunk.as_ptr() as *const u8,
                    chunk.len() * stride as usize,
                );
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
                let mut off = 0;
                for (i, &(_, n)) in PART_ATTRS.iter().enumerate() {
                    gl.enable_vertex_attrib_array(i as u32);
                    gl.vertex_attrib_pointer_f32(i as u32, n, glow::FLOAT, false, stride, off);
                    off += n * 4;
                }
                let quads = chunk.len() / 4;
                gl.draw_elements(glow::TRIANGLES, quads as i32 * 6, glow::UNSIGNED_SHORT, 0);
                for i in 0..PART_ATTRS.len() {
                    gl.disable_vertex_attrib_array(i as u32);
                }
                self.stats.draws += 1;
            }
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            self.back_to_target(gl);
        }
        self.part_verts.clear();
    }

    /// Draws the quads gathered since the last flush, in one call, after
    /// working out the shapes they use.
    fn flush(&mut self, gl: &glow::Context) {
        self.raster_shapes(gl);
        if self.verts.is_empty() {
            return;
        }
        let stride = std::mem::size_of::<Vertex>() as i32;
        // SAFETY: `verts` is plain data (repr(C) floats and bytes); the
        // attributes below match its layout; the index buffer covers
        // MAX_QUADS quads and `verts` holds at most that many.
        unsafe {
            gl.use_program(Some(self.batch.id));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.shapes.tex));
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
        if area.is_empty() {
            return;
        }
        self.stats.shaded_px += area.w as usize * area.h as usize;
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

    /// The renderer's own frame as the panel takes it: native rows top to
    /// bottom, one XRGB8888 word per pixel. Waits for the GPU to finish.
    pub fn read_native(&self, gpu: &Gpu) -> Vec<u32> {
        self.frame.read(gpu, self.native.0, self.native.1)
    }

    /// `target` as the panel takes it (see [`Renderer::read_native`]).
    pub fn read_target(&self, gpu: &Gpu, target: Target) -> Vec<u32> {
        target.read(gpu, self.native.0, self.native.1)
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
pub mod kms;
pub mod scanout;

#[cfg(test)]
mod tests;
