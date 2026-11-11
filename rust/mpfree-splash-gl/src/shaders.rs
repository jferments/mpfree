//! GLSL ES 3.00 shaders for the startup screen.

pub const NOISE: &str = r#"
float hash(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}
float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}
float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    mat2 turn = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < 5; i++) {
        v += a * noise(p);
        p = turn * p * 2.03 + 17.1;
        a *= 0.5;
    }
    return v;
}
"#;

/// A full-screen triangle pair; `uv` runs 0..1 with (0, 0) at the bottom left.
pub const FULLSCREEN_VS: &str = r#"#version 300 es
layout(location = 0) in vec2 corner;
out vec2 uv;
void main() {
    uv = corner * 0.5 + 0.5;
    gl_Position = vec4(corner, 0.0, 1.0);
}
"#;

/// The galaxy's disc, face on, drawn once into a square texture.
/// `p` is in galaxy radii, -1..1; `heart` is the heart's radius in galaxy radii.
pub const DISC_FS: &str = r#"
uniform float heart;
uniform float wind;
uniform vec4 arms;
in vec2 uv;
out vec4 color;
const float PI = 3.14159265;
void main() {
    vec2 p = uv * 2.0 - 1.0;
    float r = length(p);
    float a = atan(p.y, p.x);
    float rn = clamp((r - heart) / (1.0 - heart), 0.0, 1.0);
    float lr = log(max(r, 1e-3) / heart);
    // Ragged arms: the phase wanders with large-scale noise.
    float psi = a - wind * lr + 0.45 * (fbm(p * 2.5 + 4.0) - 0.5);
    float k = floor(psi / (PI * 0.5) + 0.5);
    float d = psi - k * PI * 0.5;
    int ki = int(mod(k, 4.0));
    float amp = ki == 0 ? arms.x : ki == 1 ? arms.y : ki == 2 ? arms.z : arms.w;
    float w = 0.15 + 0.22 * rn;
    float arm = amp * exp(-(d * d) / (w * w));
    // Dark dust lanes along the inner edge of each arm, broken up by noise.
    float dd = d - w * 0.7;
    float lane = amp * exp(-(dd * dd) / (w * w * 0.12)) * smoothstep(0.35, 0.7, fbm(p * 7.0 + 3.1));
    float env = exp(-rn * 2.1) * smoothstep(heart * 0.7, heart * 1.15, r) * (1.0 - smoothstep(0.88, 1.0, r));
    float grain = 0.55 + 0.9 * noise(p * 140.0) * noise(p * 47.0 + 9.0);
    float knots = smoothstep(0.66, 0.8, fbm(p * 42.0 + 1.7)) * arm;
    vec3 young = mix(vec3(1.0, 0.86, 0.7), vec3(0.42, 0.62, 1.0), smoothstep(0.0, 0.35, rn));
    vec3 col = env * (vec3(0.13, 0.11, 0.16) * grain
                      + arm * grain * young * 1.25
                      + knots * vec3(1.0, 0.38, 0.62) * 1.1
                      + knots * smoothstep(0.75, 0.85, fbm(p * 31.0)) * vec3(0.5, 0.8, 1.0) * 1.5);
    col *= 1.0 - 0.8 * clamp(lane * env * 3.0, 0.0, 1.0);
    // Wispy dust everywhere on the disc.
    col *= mix(1.0, 0.6, smoothstep(0.5, 0.72, fbm(p * 11.0 + 7.7)) * smoothstep(heart, heart * 1.6, r));
    // The bulge: a gold glow with a white-hot core.
    float b = r / (heart * 1.05);
    col += exp(-b * b) * vec3(1.0, 0.78, 0.48) * 1.05;
    float c = r / (heart * 0.35);
    col += exp(-c * c) * vec3(1.0, 0.95, 0.85) * 0.7;
    col = 1.0 - exp(-col * 1.7);
    color = vec4(col, 1.0);
}
"#;

/// Deep space behind everything, drawn once at the screen's size.
pub const SKY_FS: &str = r#"
uniform vec2 size;
in vec2 uv;
out vec4 color;
void main() {
    vec2 p = (uv - 0.5) * vec2(size.x / size.y, 1.0);
    float n1 = fbm(p * 2.2 + 11.0);
    float n2 = fbm(p * 3.7 - 5.0);
    vec3 col = vec3(0.012, 0.012, 0.035);
    col += vec3(0.22, 0.05, 0.30) * pow(smoothstep(0.4, 0.8, n1), 2.0) * 0.8;
    col += vec3(0.02, 0.18, 0.26) * pow(smoothstep(0.45, 0.85, n2), 2.0) * 0.7;
    float vignette = smoothstep(1.2, 0.3, length(p));
    color = vec4(col * vignette, 1.0);
}
"#;

pub const TEXTURE_FS: &str = r#"#version 300 es
precision highp float;
uniform sampler2D tex;
in vec2 uv;
out vec4 color;
void main() { color = texture(tex, uv); }
"#;

/// The disc, turned and tilted: `corner` is -1..1 in the disc's own plane.
pub const DISC_VS: &str = r#"#version 300 es
layout(location = 0) in vec2 corner;
uniform vec2 screen;
uniform float radius;
uniform float tilt;
uniform float spin;
out vec2 uv;
void main() {
    uv = corner * 0.5 + 0.5;
    float c = cos(spin), s = sin(spin);
    vec2 q = vec2(c * corner.x - s * corner.y, s * corner.x + c * corner.y);
    vec2 px = screen * 0.5 + vec2(q.x, q.y * tilt) * radius;
    gl_Position = vec4(px / screen * 2.0 - 1.0, 0.0, 1.0);
    gl_Position.y = -gl_Position.y;
}
"#;

/// Stars as soft round points. kind 0: sky (pos in 0..1 of the screen);
/// 1: disc star (pos in galaxy radii); 2: heart star (turns faster).
pub const STAR_VS: &str = r#"#version 300 es
layout(location = 0) in vec2 pos;
layout(location = 1) in vec4 tint;   // rgb, brightness
layout(location = 2) in vec3 info;   // kind, size, twinkle phase
uniform vec2 screen;
uniform float radius;
uniform float tilt;
uniform float spin;
uniform float heart_speed;
uniform float time;
uniform float scale;
out vec4 v_tint;
void main() {
    vec2 px;
    if (info.x < 0.5) {
        px = pos * screen;
    } else {
        float a = info.x < 1.5 ? spin : spin * heart_speed;
        float c = cos(a), s = sin(a);
        vec2 q = vec2(c * pos.x - s * pos.y, s * pos.x + c * pos.y);
        px = screen * 0.5 + vec2(q.x, q.y * tilt) * radius;
    }
    float twinkle = 0.65 + 0.35 * sin(time * (1.0 + 2.0 * fract(info.z * 7.3)) + info.z * 6.2831);
    v_tint = vec4(tint.rgb, tint.a * twinkle);
    gl_PointSize = info.y * scale;
    vec2 ndc = px / screen * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
"#;

pub const STAR_FS: &str = r#"#version 300 es
precision mediump float;
in vec4 v_tint;
out vec4 color;
void main() {
    vec2 d = gl_PointCoord * 2.0 - 1.0;
    float r2 = dot(d, d);
    float core = exp(-r2 * 7.0);
    float spike = exp(-abs(d.x) * 14.0) * exp(-abs(d.y) * 2.5) + exp(-abs(d.y) * 14.0) * exp(-abs(d.x) * 2.5);
    float v = (core + 0.25 * spike * step(0.8, v_tint.a)) * v_tint.a;
    color = vec4(v_tint.rgb * v, 0.0);
}
"#;

/// The title: R = letter, G = outline, B = glow, from the CPU raster.
pub const TITLE_VS: &str = r#"#version 300 es
layout(location = 0) in vec2 corner;
uniform vec2 screen;
uniform vec2 box;   // title size in pixels
out vec2 uv;
void main() {
    uv = vec2(corner.x * 0.5 + 0.5, 0.5 - corner.y * 0.5);
    vec2 px = screen * 0.5 + corner * box * 0.5;
    vec2 ndc = px / screen * 2.0 - 1.0;
    gl_Position = vec4(ndc, 0.0, 1.0);
}
"#;

pub const TITLE_FS: &str = r#"#version 300 es
precision highp float;
uniform sampler2D tex;
uniform float time;
uniform vec2 tex_size;
uniform float ppu;        // pixels per glyph unit
uniform float pad;        // pixels of padding around the glyphs
uniform float line_pitch; // glyph units from one line's top to the next
uniform float cap;        // cap height, glyph units
in vec2 uv;
out vec4 color;
void main() {
    vec3 m = texture(tex, uv).rgb;
    float ink = m.r, halo = m.g, glow = m.b;
    // Height within the current line, 0 at the top of the capitals, 1 at the baseline.
    float unit_y = (uv.y * tex_size.y - pad) / ppu;
    float v = clamp(mod(unit_y, line_pitch) / cap, 0.0, 1.0);
    // Chrome: white sky above a horizon line, deep cyan to blue below it.
    vec3 upper = mix(vec3(1.0), vec3(0.72, 0.93, 1.0), v / 0.52);
    vec3 lower = mix(vec3(0.15, 0.78, 1.0), vec3(0.18, 0.25, 0.95), (v - 0.52) / 0.48);
    vec3 fill = v < 0.52 ? upper : lower;
    fill += vec3(0.9) * exp(-pow((v - 0.52) * 30.0, 2.0)) * 0.6;
    // A highlight sweeping across every few seconds.
    float x = uv.x * tex_size.x / ppu;
    float sweep = fract(time * 0.12) * (tex_size.x / ppu + 30.0) - 15.0;
    fill += vec3(0.6) * exp(-pow((x - sweep + (unit_y * 0.6)) * 0.35, 2.0));
    vec3 outline = vec3(0.01, 0.01, 0.06);
    vec3 glow_col = mix(vec3(0.15, 0.75, 1.0), vec3(0.85, 0.25, 1.0), 0.5 + 0.5 * sin(time * 0.7));
    vec3 rgb = fill * ink + outline * (1.0 - ink) * halo + glow_col * glow * (1.0 - halo) * 0.9;
    float a = ink + (1.0 - ink) * halo;
    color = vec4(rgb, a);
}
"#;
