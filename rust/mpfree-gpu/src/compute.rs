//! A GPU compute round trip: how long a small job takes to go to the GPU
//! and come back, against the same job on one CPU core, and how close the
//! two answers are.
//!
//! The job is an FIR filter over a stereo block, the shape of an audio
//! effect: each output sample is the sum of `taps` input samples times the
//! filter's taps. One round trip uploads the block, runs the compute
//! shader, and reads the answer back, as an effect handed to the GPU would
//! (the performance plan's "GPU compute"). It needs OpenGL ES 3.1
//! ([`Gpu::surfaceless_es`]`(3, 1)`); the MPC's Mesa reports 3.1.
//!
//! The GPU adds in a different order and with fused multiply-adds, so its
//! answer is close to the CPU's but not bit for bit; the error is reported.

use std::time::Instant;

use glow::HasContext;

use crate::Gpu;

const FIR_CS: &str = r"#version 310 es
layout(local_size_x = 64) in;
// Interleaved stereo: (taps - 1) frames of history, then the block.
layout(std430, binding = 0) readonly buffer In { float x[]; };
layout(std430, binding = 1) readonly buffer Taps { float h[]; };
layout(std430, binding = 2) writeonly buffer Out { float y[]; };
uniform int frames;
uniform int taps;
void main() {
    int i = int(gl_GlobalInvocationID.x);
    if (i >= frames * 2) {
        return;
    }
    int ch = i & 1;
    int n = i >> 1;
    float acc = 0.0;
    for (int k = 0; k < taps; k++) {
        acc += h[k] * x[((n + taps - 1 - k) << 1) | ch];
    }
    y[i] = acc;
}
";

/// Round-trip times for one job size, in microseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct RoundTrip {
    pub frames: usize,
    pub taps: usize,
    pub iterations: usize,
    pub gpu_p50_us: f64,
    pub gpu_p99_us: f64,
    pub gpu_max_us: f64,
    /// The same job on this CPU core, median.
    pub cpu_p50_us: f64,
    /// The largest difference between the GPU's and the CPU's samples.
    pub max_error: f32,
}

/// The CPU's answer: `y[2n + ch] = sum_k h[k] * x[2(n + taps - 1 - k) + ch]`.
pub fn fir_cpu(x: &[f32], h: &[f32], frames: usize, y: &mut [f32]) {
    let taps = h.len();
    for n in 0..frames {
        for ch in 0..2 {
            let mut acc = 0.0f32;
            for (k, hk) in h.iter().enumerate() {
                acc += hk * x[2 * (n + taps - 1 - k) + ch];
            }
            y[2 * n + ch] = acc;
        }
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn as_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Runs the FIR job `iterations` times (after 10 untimed warm-up rounds)
/// on `gpu`, which must be an OpenGL ES 3.1 context, and on the CPU.
pub fn fir_round_trip(
    gpu: &Gpu,
    frames: usize,
    taps: usize,
    iterations: usize,
) -> Result<RoundTrip, String> {
    let gl = &gpu.gl;
    let n_in = 2 * (frames + taps - 1);
    let n_out = 2 * frames;
    // A fixed, made-up signal and filter: the answer is checked, not heard.
    let x: Vec<f32> = (0..n_in)
        .map(|i| ((i * 7919 % 2003) as f32 / 1001.5) - 1.0)
        .collect();
    let h: Vec<f32> = (0..taps)
        .map(|k| ((k * 104729 % 997) as f32 / 997.0 - 0.5) / taps as f32)
        .collect();
    let mut want = vec![0.0f32; n_out];
    fir_cpu(&x, &h, frames, &mut want);
    // SAFETY: GL calls on the current context with buffers this function
    // creates, sizes and deletes; the mapped range is read within its size.
    unsafe {
        let shader = gl.create_shader(glow::COMPUTE_SHADER)?;
        gl.shader_source(shader, FIR_CS);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            return Err(format!("the compute shader did not compile: {log}"));
        }
        let program = gl.create_program()?;
        gl.attach_shader(program, shader);
        gl.link_program(program);
        gl.delete_shader(shader);
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(format!("the compute program did not link: {log}"));
        }
        gl.use_program(Some(program));
        let loc = |name| gl.get_uniform_location(program, name);
        gl.uniform_1_i32(loc("frames").as_ref(), frames as i32);
        gl.uniform_1_i32(loc("taps").as_ref(), taps as i32);
        let mut buffers = Vec::new();
        for (binding, bytes, usage) in [
            (0u32, n_in * 4, glow::DYNAMIC_DRAW),
            (1, taps * 4, glow::STATIC_DRAW),
            (2, n_out * 4, glow::DYNAMIC_READ),
        ] {
            let b = gl.create_buffer()?;
            gl.bind_buffer(glow::SHADER_STORAGE_BUFFER, Some(b));
            gl.buffer_data_size(glow::SHADER_STORAGE_BUFFER, bytes as i32, usage);
            gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, binding, Some(b));
            buffers.push(b);
        }
        gl.bind_buffer(glow::SHADER_STORAGE_BUFFER, Some(buffers[1]));
        gl.buffer_sub_data_u8_slice(glow::SHADER_STORAGE_BUFFER, 0, &as_bytes(&h));
        let x_bytes = as_bytes(&x);
        let groups = n_out.div_ceil(64) as u32;
        let mut got = vec![0.0f32; n_out];
        let mut times = Vec::with_capacity(iterations);
        let mut failed = None;
        for i in 0..iterations + 10 {
            let t = Instant::now();
            gl.bind_buffer(glow::SHADER_STORAGE_BUFFER, Some(buffers[0]));
            gl.buffer_sub_data_u8_slice(glow::SHADER_STORAGE_BUFFER, 0, &x_bytes);
            gl.dispatch_compute(groups, 1, 1);
            gl.memory_barrier(glow::BUFFER_UPDATE_BARRIER_BIT);
            gl.bind_buffer(glow::SHADER_STORAGE_BUFFER, Some(buffers[2]));
            let p = gl.map_buffer_range(
                glow::SHADER_STORAGE_BUFFER,
                0,
                (n_out * 4) as i32,
                glow::MAP_READ_BIT,
            );
            if p.is_null() {
                failed = Some(format!(
                    "glMapBufferRange failed (GL error {:#x})",
                    gl.get_error()
                ));
                break;
            }
            let out = std::slice::from_raw_parts(p as *const u8, n_out * 4);
            for (g, b) in got.iter_mut().zip(out.as_chunks::<4>().0) {
                *g = f32::from_le_bytes(*b);
            }
            gl.unmap_buffer(glow::SHADER_STORAGE_BUFFER);
            if i >= 10 {
                times.push(t.elapsed().as_secs_f64() * 1e6);
            }
        }
        for b in buffers {
            gl.delete_buffer(b);
        }
        gl.use_program(None);
        gl.delete_program(program);
        if let Some(e) = failed {
            return Err(e);
        }
        let err = gl.get_error();
        if err != glow::NO_ERROR {
            return Err(format!("GL error {err:#x} in the compute round trip"));
        }
        let max_error = got
            .iter()
            .zip(&want)
            .map(|(g, w)| (g - w).abs())
            .fold(0.0f32, f32::max);
        times.sort_by(f64::total_cmp);
        let mut cpu = Vec::with_capacity(iterations);
        let mut y = vec![0.0f32; n_out];
        for _ in 0..iterations.max(1) {
            let t = Instant::now();
            fir_cpu(std::hint::black_box(&x), &h, frames, &mut y);
            std::hint::black_box(&y);
            cpu.push(t.elapsed().as_secs_f64() * 1e6);
        }
        cpu.sort_by(f64::total_cmp);
        Ok(RoundTrip {
            frames,
            taps,
            iterations,
            gpu_p50_us: percentile(&times, 0.5),
            gpu_p99_us: percentile(&times, 0.99),
            gpu_max_us: *times.last().unwrap_or(&0.0),
            cpu_p50_us: percentile(&cpu, 0.5),
            max_error,
        })
    }
}
