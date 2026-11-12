# mpfree-gpu

Plays mpfree-ui display lists on the GPU with OpenGL ES 2, straight into the
panel's own orientation. It is step G4 of the performance plan's "GPU drawing
and compute" tab, and the renderer behind
[the UI's GPU seam](../../docs/dev/design/ui-architecture.md#drawing-on-the-gpu).

- Every screen draws through `mpfree_ui::Canvas`; `mpfree_ui::record::Recorder`
  keeps a frame's calls as a list. `Renderer::play` draws that list into a
  texture laid out as the panel is (800 x 1280 on the MPC One, turned 270
  degrees), so nothing is turned afterwards.
- `Gpu::surfaceless` opens EGL on Mesa's surfaceless platform: no window and
  no display, as the MPC's offscreen GPU tests did on 9 Nov. It loads
  `libEGL.so.1` at run time, so this crate links against no GPU library and
  builds anywhere; it fails, saying why, where there is none.
- The frame stays between lists, as the software canvas does, so a frame
  only redraws what changed.
- Shapes and text go in one draw call. Masks (glyphs, stroke masks) are
  uploaded once into a shared texture, the atlas (2048 x 1024, 2 MiB), and
  one shader draws both shapes and masks, so a run of them is a single
  call. Clips cost no call: each quad is cut to the clip on whole pixels
  before it is drawn. Every one of the 59 sample pages draws in one call
  (it was 672 at the median before the atlas). An image is a call of its
  own. When the atlas is full the renderer draws what is waiting, clears
  it, and uploads masks again as they are used; a mask bigger than the
  atlas is drawn alone. Both are counted in `DrawStats` (`atlas_cleared`,
  `masks_alone`) and the benchmark prints them; neither happened on the
  sample pages.

It is its own Cargo workspace, outside `rust/Cargo.toml`, because it is a
normal dynamically linked program (it loads the system's EGL), while the app
is one static program. On the device it is meant to run as its own process,
so a crash in the GPU driver cannot stop the sound.

## Test it on the workstation

Needs Mesa's EGL (Ubuntu: `sudo apt-get install libegl1 libegl-mesa0 libgles2`).

```bash
cargo test --manifest-path rust/mpfree-gpu/Cargo.toml --locked -- --nocapture
```

The tests draw a scene that uses all six canvas calls (fills, see-through
fills, rounded rectangles, outlines, text in three faces, images, clips)
both ways and compare the GPU's frame with the bytes `mpfree_ui::fb` sends
to the panel, for the MPC's screen and for every panel turn. On Mesa's
llvmpipe (2026-11-12) 7,455 of 1,024,000 pixels differ, 51 of them by more
than one level, and none by more than two: the GPU blends in floating point,
the software canvas in integers.

## The benchmark (the plan's step G2)

`mpfree-gpu-bench` times drawing on the GPU against software on the machine
it runs on, and a GPU compute round trip:

```bash
cargo build --release --manifest-path rust/Cargo.toml -p mpfree-app
rust/target/release/mpfree --display-lists /tmp/pages        # every sample page, two frames each
cargo run --release --manifest-path rust/mpfree-gpu/Cargo.toml --bin mpfree-gpu-bench -- describe
cargo run --release --manifest-path rust/mpfree-gpu/Cargo.toml --bin mpfree-gpu-bench -- screens /tmp/pages
cargo run --release --manifest-path rust/mpfree-gpu/Cargo.toml --bin mpfree-gpu-bench -- compute
```

- `screens DIR [--frames N]` plays each page at the MPC's size (an 800 x
  1280 panel turned 270 degrees): its first frame with every upload, the
  whole page again (what a page change costs), the redraw a tick later, and
  reading the frame back; then the same lists in software, the software
  copy to the panel, and how far the GPU's picture is from the software one.
- `compute [--iterations N]` runs an FIR filter over a stereo block on the
  GPU (upload, run, read back; OpenGL ES 3.1 compute shader) and on one CPU
  core, for one period with 64 and 1024 taps and for 32 periods.

For the MPC it is built for ARM against the MPC's C library (dynamically
linked: it opens the MPC's `libEGL.so.1` and `libGLESv2.so.2` itself):

```bash
sudo apt-get install gcc-arm-linux-gnueabihf                  # once
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --release --manifest-path rust/mpfree-gpu/Cargo.toml \
  --target armv7-unknown-linux-gnueabihf --bin mpfree-gpu-bench
```

That build loads with `/lib/ld-linux-armhf.so.3`, links only `libc.so.6` and
`libgcc_s.so.1`, and asks for nothing newer than glibc 2.34 (the MPC has
2.39). `tools/mpc-one/host_gpu_mpc.py` checks all that, copies it and the
pages into the test build's RAM, and runs it there, offscreen. On
2026-11-12 the ARM build ran every step under qemu-arm with Ubuntu's ARM
Mesa (llvmpipe), with the GPU's pictures within 2 levels of software; the
times there are an emulator's and mean nothing.

## Not yet

- The renderer as its own process on the MPC, fed the display list through
  shared memory (the wire format in `mpfree_ui::record` is that feed).
- Showing the frame on the panel without reading it back (DRM/KMS page
  flips): it changes the display settings in RAM and waits for Jay's go.
