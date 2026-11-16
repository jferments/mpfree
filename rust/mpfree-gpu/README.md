# mpfree-gpu

Plays mpfree-ui display lists on the GPU with OpenGL ES 2, straight into the
panel's own orientation and, through DRM/KMS page flips, straight into the
buffers the panel shows. `mpfree-gpu-screen` draws the app's screen that way
in a process of its own (step G4 of the performance plan's "GPU drawing and
compute" tab). It is the renderer behind
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
- Vector shapes (the synth and FX curves, knob rings, graphs:
  `mpfree_ui::Shape`) come as their parts (segments, discs, arcs, areas
  under a line), not as pixels. The first time a list names one, the GPU
  works out its coverage itself into a second atlas (1024 x 1024 RGBA, 4
  MiB: an atlas the GPU can draw into), all of a frame's new shapes in one
  draw call, with MAX blending (`GL_EXT_blend_minmax`) so overlapping parts
  take the larger coverage as software does. After that a shape draws like
  a mask, in the same call as everything else. On the sample pages this cut
  a synth page's first list from 488 KB to 46 KB (the FX chain's from 220
  KB to 52 KB, all 73 pages from 5.8 MB to 4.3 MB), the app no longer
  rasterises curves on the screen thread while the GPU draws, and moving a
  curve with a knob uploads nothing. Shapes come out within one level of
  software's on Mesa's software GPU. Without MAX blending, or for a shape
  bigger than its atlas, the renderer takes software's coverage and draws
  it as a mask. `DrawStats` counts `shapes_drawn` and `shapes_cleared`, and
  the benchmark prints both.
- Both atlases pack onto shelves of about the same height, so text of one
  size shares a shelf. When no shelf of about its height has room and there
  is no room left for a new shelf, a mask or shape goes on the lowest taller
  shelf that has room before the atlas counts as full. Since the synth page
  redesign (train 47), the FM and Track pages draw seven 388 x 191 layers of
  one graph and then three small knob rings; the knobs found no shelf, so
  the shapes' atlas was cleared halfway through each of those pages (four
  draw calls instead of two, and every layer worked out again whenever it
  was drawn after a knob). Now all 79 sample pages take at most two calls,
  and the shapes' atlas is never cleared.
- Most pixels are plain: a fill, or the inside of a rounded rectangle or an
  outline. A quad marked plain skips the shader's coverage maths and writes
  its colour. A rounded rectangle is drawn as three plain bands and four
  shaded corners, and an outline as four plain edges and four shaded corners
  (a whole shaded quad only when it is too small to split). The picture is
  the same; on the sample pages the shaded pixels per page fell from about
  3 million to 50,000 to 100,000. `DrawStats` counts plain, shaded and
  copied pixels.
- Only what changed is drawn: `list_damage` finds the rectangles a list
  touches (up to 16, merged), and drawing is cut to them (a scissor), so the
  GPU's fragment work covers the change, not the screen.

It is its own Cargo workspace, outside `rust/Cargo.toml`, because it is a
normal dynamically linked program (it loads the system's EGL), while the app
is one static program. On the device it is meant to run as its own process,
so a crash in the GPU driver cannot stop the sound.

## Straight to the panel (page flips)

Live test 3 (13 Nov) timed the GPU's way to the panel at 63 ms a frame
against software's 32 ms: the GPU drew into its own memory, the frame was
read back and copied to `/dev/fb0`, and every pixel paid for coverage
maths. `kms` and `scanout` remove the read-back and the copy:

- `kms::KmsDisplay` (raw DRM ioctls on `/dev/dri/card0`, no libdrm) finds
  the lit connector and its CRTC, makes three screen buffers in RAM (dumb
  buffers at the panel's native 800 x 1280, XRGB8888) and hands each to the
  GPU as a dma-buf. The first frame shown takes the display (DRM master)
  and points the CRTC at a buffer in the mode it already runs; after that
  each frame is a page flip, and the next frame waits for the flip to
  happen. It saves the CRTC as it found it (framebuffer, position, mode) and
  puts it back when it is closed or dropped. If the process is killed
  instead, the kernel gives the screen back to the framebuffer when the
  display's last file closes; a power cycle undoes everything. Nothing is
  stored.
- `scanout::import_dmabuf` turns a screen buffer into a texture the GPU
  draws into (`EGL_EXT_image_dma_buf_import`), and `Scanout` shows two in
  turn: while one is on the panel the other is drawn, so nothing is seen
  half drawn. Each buffer keeps the rectangles it missed while the other
  was shown, and copies them from the front buffer in the same pass before
  the new list is drawn.
- Memory row 0 is the panel's top row: the picture is drawn in the panel's
  own portrait orientation, with no flip.

`mpfree-gpu-bench flip` and `mpfree-gpu-screen` both check, before anything
is shown, that the GPU's picture is in the screen buffer's own memory as
drawn (read back once through the buffer's mapping), and stop if not.

## The screen process (G4)

`mpfree-gpu-screen` draws the app's screen. The app (`mpfree app
--gpu-screen PROGRAM`, or `play`) starts it with `--rotation`, sends each
frame's display list down its stdin in the wire format of
`mpfree_ui::record`, and reads one answer per frame from its stdout
(`mpfree_ui::gpu_screen` has the protocol and the client):

- `ready NWxNH LWxLH HOW` when it is set up;
- `shown SEQ DRAW_US GPU_US WAIT_US PLAIN SHADED COPIED` for a frame shown;
- `same SEQ` for a frame that changed nothing;
- `fail: REASON` when it stops (it puts the screen back first).

A crash or hang in the GPU's driver ends this process, never the app or its
sound. The app notices the process gone, failed, or 1.5 s without an
answer, stops it, and draws in software from then on: counted, logged and
shown on the Timing panel's Screen row (`rust/mpfree-app/src/drawing.rs`).
It does not try the GPU again until the app starts again.

A pipe, not shared memory: after a page's first frame a list is 0 to 62 KB,
which a pipe copies in microseconds, and a pipe brings order and end of file
(the app gone) for free.

`--offscreen [--size WxH] [--last-frame FILE]` draws into the GPU's own
memory and shows nothing, for tests. `--test-fail-after N` and
`--test-stall-after N` fail or hang after N frames, for testing the app's
fallback. `tests/screen_process.rs` runs it as the app does and checks that
every frame is answered (also when several arrive at once), that the last
frame is software's picture (at most 3 levels apart; 1 on llvmpipe), and
that a failure and a hang are each noticed. `MPFREE_GPU_SCREEN=PROGRAM`
points those tests at another build: on 13 Nov they passed against the ARM
build run under qemu-arm with Ubuntu's ARM Mesa (llvmpipe).

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
  path's turn into the panel's layout, and how far the GPU's picture is
  from the software one.
- `compute [--iterations N]` runs an FIR filter over a stereo block on the
  GPU (upload, run, read back; OpenGL ES 3.1 compute shader) and on one CPU
  core, for one period with 64 and 1024 taps and for 32 periods.
- `flip DIR [--hold-ms N] [--frames N] [--card PATH]` draws on the GPU
  straight into buffers the panel shows (above). **It changes what the
  screen shows, in RAM.** With the screen untouched it checks the first
  page in screen buffer 0's memory and the GPU's picture against
  software's; then it prints `SCREEN CHANGES NOW`, shows each page for N ms
  (1500), and times the whole page and a tick's redraw N times (60) the
  GPU's way (into the next buffer, then a flip) and software's (draw, turn
  into the panel's layout, copy into a screen buffer that is never shown,
  each timed on its own), with the CPU each takes. It puts the screen back and says `the screen is back as it was`,
  also on Ctrl-C, SIGTERM, SIGHUP or a failure.
- `panel DIR [--hold-ms N] [--frames N]` puts the GPU's pictures on the
  panel: each page is drawn on the GPU, read back and written to
  `/dev/fb0` (through `mpfree_ui::fb::FbTarget`, as the app's screen
  writes: whole frames, `0,0` to the sysfs `pan` file, nothing created, no
  display setting changed) and held for N ms (1500). It reads the
  framebuffer back once to check it holds what was written. Then the
  busiest page is drawn and copied to the panel N times (60) the GPU's way
  (draw, read back, copy) and software's (draw, turn, copy), timed.
  `--fb PATH --sysfs DIR` name another framebuffer, for tests: with a
  plain 4,096,000-byte file and a folder holding `modes`
  (`U:800x1280p-0`), `stride` (3200), `bits_per_pixel` (32) and an empty
  `pan`, it runs on any workstation.

For the MPC it is built for ARM against the MPC's C library (dynamically
linked: it opens the MPC's `libEGL.so.1` and `libGLESv2.so.2` itself):

```bash
sudo apt-get install gcc-arm-linux-gnueabihf                  # once
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --release --manifest-path rust/mpfree-gpu/Cargo.toml \
  --target armv7-unknown-linux-gnueabihf --bins
```

Both programs (`mpfree-gpu-bench` and `mpfree-gpu-screen`) load with
`/lib/ld-linux-armhf.so.3`, link only `libc.so.6` and `libgcc_s.so.1`, and
ask for nothing newer than glibc 2.34 (the MPC has 2.39).
`tools/mpc-one/host_gpu_mpc.py` checks all that, copies the benchmark and
the pages into the test build's RAM, and runs it there: offscreen, then
(step 6) on the panel, then, only with `--flip` and when no program has the
display open, `flip` (step 7). On 2026-11-12 the ARM build ran every step under qemu-arm
with Ubuntu's ARM Mesa (llvmpipe), with the GPU's pictures within 2 levels
of software and, for the panel step, a fake framebuffer that read back what
was written; the times there are an emulator's and mean nothing.

## Not yet

- On the MPC: `flip` (step 7 of the device tool) is block Q9 (G0 to G2)
  of the [3.10.1 device round](../../docs/dev/hardware/firmware-3.10.1-qualification.md),
  run from the soft-boot USB stick. Its GPU `whole p50` against software's
  32 ms (live test 3) says whether drawing on the GPU pays, and its software
  line says whether the turn into the panel's layout or the copy costs more.
- The app with `--gpu-screen` has not run on the MPC. The test stick does
  not carry `mpfree-gpu-screen` yet (the stick builder copies only the
  app), so that run needs a stick-builder change and its own runbook block.
- Plain pixels are still drawn about three times over on a busy page
  (panels over the background); drawing front to back with a depth test
  could skip the hidden ones.
- The third screen buffer is made but not used; a frame never waits for
  more than one flip today.
