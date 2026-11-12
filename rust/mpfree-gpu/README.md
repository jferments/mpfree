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
  only redraws what changed. Runs of rectangles and rounded rectangles are
  one draw call; each text mask and image is uploaded once, as a texture.

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

## Not yet

- Running on the MPC: the next step is a timing run at full size, offscreen
  (nothing on the device changes), cross-built for its C library.
- Showing the frame on the panel without reading it back (DRM/KMS page
  flips): it changes the display settings in RAM and waits for Jay's go.
- A glyph atlas, so text is one draw call per run instead of one per glyph.
