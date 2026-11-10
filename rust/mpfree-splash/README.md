# mpfree-splash

The MPFree startup screen: an outer-space scene with a spinning two-armed
spiral galaxy and the words **MPCFree OS v0.1** in block letters at its centre.
It is drawn with [ratatui](https://ratatui.rs) (MIT).

Every frame is a grid of upper-half-block cells (`▀`), each holding two
square pixels: the top one in the foreground colour, the bottom one in the
background colour. That lets the same scene run in two places:

- **A desktop terminal** (crossterm backend, needs 24-bit colour). For
  previewing on the workstation.
- **The MPC One's framebuffer**, through `fb::FramebufferBackend`, a ratatui
  backend that draws each cell as 8x16 pixels straight into `/dev/fb0`'s
  layout. Both of Akai's kernels have no text console on the screen, so a
  normal terminal backend has nothing to draw on. The geometry and rotation
  code follows `mpfree-screen-log` from the kexec test boot: 800x1280
  XRGB8888, stride 3200, rotation 270 (left side up), so the viewer sees
  1280x800, which is 160x50 cells or 160x100 galaxy pixels.

## Modes

| Command | What it does | Touches |
| --- | --- | --- |
| `mpfree-splash [--seconds N]` | Animates in this terminal. q, Esc or Ctrl-C quits. | The terminal only |
| `mpfree-splash --preview DIR [--frames N] [--every S]` | Renders frames through the framebuffer backend at the MPC One's exact geometry and saves what the viewer would see as PNGs. | Writes new files in DIR only (never overwrites) |
| `mpfree-splash --framebuffer [--seconds N] [--fps N]` | Draws on `/dev/fb0` at 20 fps by default and prints frame timings. | Writes only `/dev/fb0` (one visible frame from offset 0, no create or truncate) and `0,0` to `/sys/class/graphics/fb0/pan` once. Never writes storage. |

`--framebuffer` is for a boot with nothing else on the screen (such as the
kexec test boot). It is not for the stock firmware while Akai's app owns the
display.

## Build

Desktop:

```bash
cargo run --manifest-path rust/Cargo.toml --release --locked -p mpfree-splash
```

Static ARM executable for the MPC (same flags as `mpfree-screen-log`):

```bash
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" \
cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-splash
```

CI builds it that way and runs it under `qemu-arm` against a fake framebuffer.

`src/font.rs` is font8x8_basic (public domain), copied from `mpfree-screen-log`.
