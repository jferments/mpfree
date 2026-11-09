# MPC One Mali-T760 / Panfrost offscreen GPU qualification (2026-11-09)

## Status: GPU offscreen rendering proven; on-screen GUI not qualified

The owner ran two native **ARMv7 hard-float** tests on the physical Akai
Professional MPC One using a direct SSH connection. The original MPC interface
was not stopped, DRM/KMS master was not requested, and no display was changed.
Code and launchers are in [tools/mpc-one/gpu-qualification](../../tools/mpc-one/gpu-qualification/).

### Observed device

| Property | Observed |
| --- | --- |
| SoC | Rockchip RK3288 |
| GPU | Mali-T760 (Mesa Panfrost) |
| Kernel | 6.6.95-az01-2025-06-27-rt55, PREEMPT_RT, ARMv7 |
| EGL | 1.4 (Mesa Project) |
| OpenGL ES | 3.1 (Mesa 24.0.7) |
| Render node | `/dev/dri/renderD128` (Panfrost) |
| GPU driver library | `/usr/lib/dri/panfrost_dri.so` |
| Total memory at inventory | 2,067,188 kB (from `/proc/meminfo`) |
| Available memory at inventory | 1,357,156 kB |
| CMA | 12,288 kB total, 0 kB free during survey |

Stage 1: GBM could initialize an EGL display from `renderD128`, but the
requested GLES2 pbuffer EGLConfig was unavailable. The independently
attempted **Mesa surfaceless EGL** context successfully used
`GL_RENDERER: Mali-T760 (Panfrost)`, drew an offscreen 256×256 buffer,
read back RGBA `51 153 204 255`, and passed pixel verification with no GL
error. This proves real hardware rendering, **not** GBM pbuffer viability.

Stage 2: a surfaceless EGL context rendered an animated 800×480 RGBA8
framebuffer object with four 256-vertex waveforms, 64 moving frequency bars,
and a 16×8 step-sequencer grid, using dynamic VBO updates.

| Measurement | Observed |
| --- | ---: |
| Warmup frames | 20 |
| Sampled frames | 120 |
| Mean synchronized frame time | 2.376 ms |
| p50 | 2.361 ms |
| p95 | 2.579 ms |
| p99 | 2.742 ms |
| Maximum | 2.786 ms |
| Wall time including 12 ms pacing | 2,033.4 ms |
| VmRSS at report time | 15,096 kB |
| VmHWM | 16,512 kB |
| Result | PASS; 800×480 PPM preview produced |

The generated preview showed four colored waveforms, blue spectral bars,
and a step grid on a dark canvas. The numbers include the CPU-side submission
and a `glFinish()` completion wait; they are **not GPU-only execution times,
window swap times, vsync measurements, or real display FPS**. VmRSS is not
total GPU/driver memory. A single short run is not a stability or thermal test.

### Implications for MPFree GUI design

This result supports proceeding with GPU-accelerated, native, permissively
licensed rendering (such as SDL3/OpenGL ES or a custom OpenGL ES adapter),
**without** selecting a production GUI framework on benchmark data alone.
The actual planned logical UI is **1280×800** (M8), roughly 2.67× the pixel
count tested here; a native portrait DSI mode is 800×1280. We must measure
the full logical resolution and investigate orientation, scanout, touch
mapping, font rendering, alpha blending, GPU memory pressure, audio underruns,
thermal behavior, and long-tail frame pacing.

### Next safe experiments

1. Offscreen 1280×800 rendering with more representative text, curves,
   compositing, clipping, masks, waveform zoom, and spectrum updates.
2. Longer frame-time distributions plus process and system memory monitoring
   while the stock MPC is running normal audio workloads.
3. Keep display/input takeover behind separate recovery and reversible-handoff
   approvals described by `M8_DISPLAY_OFFSCREEN_TESTCARD.md`.

**Do not interpret an offscreen GPU PASS as permission to open KMS display
nodes, stop `acvs`, grab touch events, or switch framebuffer owners.**
