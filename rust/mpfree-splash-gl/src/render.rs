//! The OpenGL ES 3.0 renderer. All GL calls are in this file.

use crate::shaders;
use glow::HasContext;
use mpfree_splash::galaxy::{self, ARMS, HEART_SPEED, TILT, TURN_SECONDS, WIND};
use mpfree_splash::{blockfont, title};

/// The disc texture's side, in pixels. Drawn once at start.
const DISC_SIZE: i32 = 2048;

pub struct Renderer {
    gl: glow::Context,
    quad: glow::VertexArray,
    stars: glow::VertexArray,
    star_count: i32,
    disc_tex: glow::Texture,
    sky_tex: Option<glow::Texture>,
    title_tex: Option<glow::Texture>,
    textured: glow::Program,
    disc_prog: glow::Program,
    star_prog: glow::Program,
    title_prog: glow::Program,
    sky_prog: glow::Program,
    size: (i32, i32),
    /// Layout for the current size: galaxy radius and title box, in pixels.
    radius: f32,
    title_box: (f32, f32),
    title_ppu: f32,
    title_pad: f32,
}

fn compile(gl: &glow::Context, vs: &str, fs: &str) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, src);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!("shader: {}", gl.get_shader_info_log(shader)));
            }
            gl.attach_shader(program, shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!("link: {}", gl.get_program_info_log(program)));
        }
        Ok(program)
    }
}

/// A fragment shader with the version line, precision and noise functions in front.
fn with_noise(body: &str) -> String {
    format!("#version 300 es\nprecision highp float;\n{}\n{}", shaders::NOISE, body)
}

fn floats(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Repeated box blurs, close to a Gaussian, over a single channel.
fn blur(src: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
    let mut a = src.to_vec();
    let mut b = vec![0.0; a.len()];
    let r = radius as i64;
    for _ in 0..3 {
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.0;
                for dx in -r..=r {
                    let sx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                    sum += a[y * w + sx];
                }
                b[y * w + x] = sum / (2 * r + 1) as f32;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.0;
                for dy in -r..=r {
                    let sy = (y as i64 + dy).clamp(0, h as i64 - 1) as usize;
                    sum += b[sy * w + x];
                }
                a[y * w + x] = sum / (2 * r + 1) as f32;
            }
        }
    }
    a
}

/// Stars: x, y, r, g, b, brightness, kind, size, phase.
fn make_stars(heart: f32) -> Vec<f32> {
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    let mut unit = move || {
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        ((rng.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u64 << 24) as f32
    };
    let mut v = Vec::new();
    let mut push = |x: f32, y: f32, rgb: [f32; 3], b: f32, kind: f32, size: f32, phase: f32| {
        v.extend_from_slice(&[x, y, rgb[0], rgb[1], rgb[2], b, kind, size, phase]);
    };
    // The sky.
    for _ in 0..1600 {
        let (x, y) = (unit(), unit());
        let t = unit();
        let rgb = if t < 0.15 { [1.0, 0.8, 0.6] } else if t < 0.35 { [0.7, 0.8, 1.0] } else { [1.0, 1.0, 1.0] };
        let big = unit().powi(6);
        push(x, y, rgb, 0.35 + 0.65 * unit(), 0.0, 1.5 + 6.0 * big, unit());
    }
    // Along the arms (same spiral as the disc texture), with some everywhere.
    for i in 0..9000 {
        let rn = unit().powf(1.3);
        let r = heart + rn * (1.0 - heart);
        let arm = (i % 4) as f32 * std::f32::consts::FRAC_PI_2;
        let gauss = (0..4).map(|_| unit()).sum::<f32>() * 1.732 - 3.464;
        let on_arm = unit() < 0.75;
        let offset = if on_arm { arm + gauss * (0.12 + 0.18 * rn) } else { unit() * std::f32::consts::TAU };
        let a = offset + WIND * (r / heart).ln();
        let t = unit();
        let rgb = if t < 0.05 { [1.0, 0.45, 0.75] } else if t < 0.6 { [0.65, 0.78, 1.0] } else { [1.0, 0.97, 0.92] };
        let b = (0.25 + 0.75 * unit().powi(2)) * (1.2 - rn) * if on_arm { 1.0 } else { 0.5 };
        push(r * a.cos(), r * a.sin(), rgb, b * 1.6, 1.0, 2.0 + 3.5 * unit().powi(4), unit());
    }
    // The heart.
    for _ in 0..1500 {
        let r = ((0..4).map(|_| unit()).sum::<f32>() * 1.732 - 3.464).abs() * 0.45 * heart;
        let a = unit() * std::f32::consts::TAU;
        push(r * a.cos(), r * a.sin(), [1.0, 0.85, 0.6], 0.3 + 0.5 * unit(), 2.0, 1.5 + 2.0 * unit().powi(3), unit());
    }
    v
}

impl Renderer {
    pub fn new(gl: glow::Context, width: i32, height: i32) -> Result<Renderer, String> {
        unsafe {
            let textured = compile(&gl, shaders::FULLSCREEN_VS, shaders::TEXTURE_FS)?;
            let disc_prog = compile(&gl, shaders::DISC_VS, shaders::TEXTURE_FS)?;
            let star_prog = compile(&gl, shaders::STAR_VS, shaders::STAR_FS)?;
            let title_prog = compile(&gl, shaders::TITLE_VS, shaders::TITLE_FS)?;
            let sky_prog = compile(&gl, shaders::FULLSCREEN_VS, &with_noise(shaders::SKY_FS))?;
            let disc_gen = compile(&gl, shaders::FULLSCREEN_VS, &with_noise(shaders::DISC_FS))?;

            // A unit quad, as two triangles, for everything flat.
            let quad = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(quad));
            let vb = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vb));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                &floats(&[-1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0]),
                glow::STATIC_DRAW,
            );
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);

            // The disc is the same at every screen size: the heart's share of the
            // radius comes from the default 1280x800 layout.
            let (heart_px, radius_px) = galaxy::galaxy_size(1280, 800);
            let heart = heart_px / radius_px;
            let disc_tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(disc_tex));
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, DISC_SIZE, DISC_SIZE, 0, glow::RGBA,
                            glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(disc_tex), 0);
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                return Err("the disc framebuffer is incomplete".into());
            }
            gl.viewport(0, 0, DISC_SIZE, DISC_SIZE);
            gl.use_program(Some(disc_gen));
            gl.uniform_1_f32(gl.get_uniform_location(disc_gen, "heart").as_ref(), heart);
            gl.uniform_1_f32(gl.get_uniform_location(disc_gen, "wind").as_ref(), WIND);
            gl.uniform_4_f32(gl.get_uniform_location(disc_gen, "arms").as_ref(), ARMS[0], ARMS[1], ARMS[2], ARMS[3]);
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(fbo);
            gl.delete_program(disc_gen);
            gl.generate_mipmap(glow::TEXTURE_2D);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);

            // Stars.
            let data = make_stars(heart);
            let stars = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(stars));
            let sb = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(sb));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &floats(&data), glow::STATIC_DRAW);
            let stride = 9 * 4;
            for (index, size, offset) in [(0u32, 2i32, 0i32), (1, 4, 8), (2, 3, 24)] {
                gl.enable_vertex_attrib_array(index);
                gl.vertex_attrib_pointer_f32(index, size, glow::FLOAT, false, stride, offset);
            }
            gl.bind_vertex_array(None);

            let mut r = Renderer {
                gl,
                quad,
                stars,
                star_count: (data.len() / 9) as i32,
                disc_tex,
                sky_tex: None,
                title_tex: None,
                textured,
                disc_prog,
                star_prog,
                title_prog,
                sky_prog,
                size: (0, 0),
                radius: radius_px,
                title_box: (0.0, 0.0),
                title_ppu: 1.0,
                title_pad: 0.0,
            };
            r.resize(width, height)?;
            Ok(r)
        }
    }

    /// Redraws what depends on the screen size: the sky and the title.
    pub fn resize(&mut self, width: i32, height: i32) -> Result<(), String> {
        if (width, height) == self.size || width <= 0 || height <= 0 {
            return Ok(());
        }
        self.size = (width, height);
        let (w, h) = (width as usize, height as usize);
        let (_, radius) = galaxy::galaxy_size(w, h);
        self.radius = radius;
        let gl = &self.gl;
        unsafe {
            // The sky.
            if let Some(t) = self.sky_tex.take() {
                gl.delete_texture(t);
            }
            let sky = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(sky));
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, width, height, 0, glow::RGBA, glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(None));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(sky), 0);
            gl.viewport(0, 0, width, height);
            gl.disable(glow::BLEND);
            gl.use_program(Some(self.sky_prog));
            gl.uniform_2_f32(gl.get_uniform_location(self.sky_prog, "size").as_ref(), width as f32, height as f32);
            gl.bind_vertex_array(Some(self.quad));
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(fbo);
            self.sky_tex = Some(sky);

            // The title: the same lines and size rule as the character-cell
            // version, rasterised at full resolution with a soft glow.
            let (lines, cap) = galaxy::title_layout(w, h);
            let inner = title::rasterise(&lines, cap);
            // Room around the letters for the glow to fade out completely.
            let glow_r = ((cap / 9.0) as usize).max(2);
            let margin = glow_r * 4;
            let mut mask = title::TitleMask {
                width: inner.width + 2 * margin,
                height: inner.height + 2 * margin,
                ink: Vec::new(),
                halo: Vec::new(),
            };
            mask.ink = vec![0.0; mask.width * mask.height];
            mask.halo = vec![0.0; mask.width * mask.height];
            for y in 0..inner.height {
                for x in 0..inner.width {
                    let (i, o) = (y * inner.width + x, (y + margin) * mask.width + x + margin);
                    mask.ink[o] = inner.ink[i];
                    mask.halo[o] = inner.halo[i];
                }
            }
            let glow_px = blur(&mask.halo, mask.width, mask.height, glow_r);
            let mut rgba = Vec::with_capacity(mask.width * mask.height * 4);
            for i in 0..mask.width * mask.height {
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                rgba.extend_from_slice(&[q(mask.ink[i]), q(mask.halo[i]), q(glow_px[i] * 1.6), 255]);
            }
            if let Some(t) = self.title_tex.take() {
                gl.delete_texture(t);
            }
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, mask.width as i32, mask.height as i32, 0, glow::RGBA,
                            glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&rgba)));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            self.title_tex = Some(tex);
            self.title_box = (mask.width as f32, mask.height as f32);
            self.title_ppu = cap / blockfont::CAP;
            // Padding: the outline's from title::rasterise plus the glow margin.
            self.title_pad = ((mask.width as f32) - blockfont::block_size(&lines).0 * self.title_ppu) / 2.0;
        }
        Ok(())
    }

    /// Draws the frame at `seconds` into whatever framebuffer is bound.
    pub fn draw(&self, seconds: f32) {
        let gl = &self.gl;
        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
        let spin = -seconds / TURN_SECONDS * std::f32::consts::TAU;
        unsafe {
            gl.viewport(0, 0, self.size.0, self.size.1);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.quad));
            gl.active_texture(glow::TEXTURE0);

            gl.use_program(Some(self.textured));
            gl.uniform_1_i32(gl.get_uniform_location(self.textured, "tex").as_ref(), 0);
            gl.bind_texture(glow::TEXTURE_2D, self.sky_tex);
            gl.draw_arrays(glow::TRIANGLES, 0, 6);

            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE);
            let p = self.disc_prog;
            gl.use_program(Some(p));
            gl.uniform_1_i32(gl.get_uniform_location(p, "tex").as_ref(), 0);
            gl.uniform_2_f32(gl.get_uniform_location(p, "screen").as_ref(), w, h);
            gl.uniform_1_f32(gl.get_uniform_location(p, "radius").as_ref(), self.radius);
            gl.uniform_1_f32(gl.get_uniform_location(p, "tilt").as_ref(), TILT);
            gl.uniform_1_f32(gl.get_uniform_location(p, "spin").as_ref(), spin);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.disc_tex));
            gl.draw_arrays(glow::TRIANGLES, 0, 6);

            let p = self.star_prog;
            gl.use_program(Some(p));
            gl.uniform_2_f32(gl.get_uniform_location(p, "screen").as_ref(), w, h);
            gl.uniform_1_f32(gl.get_uniform_location(p, "radius").as_ref(), self.radius);
            gl.uniform_1_f32(gl.get_uniform_location(p, "tilt").as_ref(), TILT);
            gl.uniform_1_f32(gl.get_uniform_location(p, "spin").as_ref(), spin);
            gl.uniform_1_f32(gl.get_uniform_location(p, "heart_speed").as_ref(), HEART_SPEED);
            gl.uniform_1_f32(gl.get_uniform_location(p, "time").as_ref(), seconds);
            gl.uniform_1_f32(gl.get_uniform_location(p, "scale").as_ref(), (h / 800.0).max(0.5));
            gl.bind_vertex_array(Some(self.stars));
            gl.draw_arrays(glow::POINTS, 0, self.star_count);

            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            let p = self.title_prog;
            gl.use_program(Some(p));
            gl.uniform_1_i32(gl.get_uniform_location(p, "tex").as_ref(), 0);
            gl.uniform_2_f32(gl.get_uniform_location(p, "screen").as_ref(), w, h);
            gl.uniform_2_f32(gl.get_uniform_location(p, "box").as_ref(), self.title_box.0, self.title_box.1);
            gl.uniform_2_f32(gl.get_uniform_location(p, "tex_size").as_ref(), self.title_box.0, self.title_box.1);
            gl.uniform_1_f32(gl.get_uniform_location(p, "time").as_ref(), seconds);
            gl.uniform_1_f32(gl.get_uniform_location(p, "ppu").as_ref(), self.title_ppu);
            gl.uniform_1_f32(gl.get_uniform_location(p, "pad").as_ref(), self.title_pad);
            gl.uniform_1_f32(gl.get_uniform_location(p, "line_pitch").as_ref(), blockfont::CAP + blockfont::LINE_GAP);
            gl.uniform_1_f32(gl.get_uniform_location(p, "cap").as_ref(), blockfont::CAP);
            gl.bind_texture(glow::TEXTURE_2D, self.title_tex);
            gl.bind_vertex_array(Some(self.quad));
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
            gl.disable(glow::BLEND);
        }
    }

    /// Renders one frame offscreen at `width` x `height` and returns its RGB
    /// pixels, top row first.
    pub fn snapshot(&mut self, width: i32, height: i32, seconds: f32) -> Result<Vec<[u8; 3]>, String> {
        self.resize(width, height)?;
        let gl = &self.gl;
        unsafe {
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, width, height, 0, glow::RGBA, glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(None));
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            self.draw(seconds);
            let gl = &self.gl;
            let mut raw = vec![0u8; (width * height * 4) as usize];
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(0, 0, width, height, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut raw)));
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(fbo);
            gl.delete_texture(tex);
            let (w, h) = (width as usize, height as usize);
            let mut out = Vec::with_capacity(w * h);
            for y in (0..h).rev() {
                for x in 0..w {
                    let o = (y * w + x) * 4;
                    out.push([raw[o], raw[o + 1], raw[o + 2]]);
                }
            }
            Ok(out)
        }
    }

    pub fn finish(&self) {
        unsafe { self.gl.finish() }
    }
}
