# mpfree-startup

Themed startup screens for MPFree, drawn in software for the 1280x800 screen.
Each `mpfree-ui` theme names its startup screen; this crate draws three:

| Theme | Scene |
| --- | --- |
| Console | A chaotic swirl of green characters settles into "MPFree" and "OS v0.1" drawn in characters with the VT323 terminal font, shaded by density and colour; a prompt types out. Scanlines and bloom. |
| Synthwave | A striped sun rises over wavy neon hills and a racing grid; "MPFree OS" in MPFree's own blade letters (angular, leaning strokes with torn, jagged starts and knife-point ends), yellow to pink over a hard shadow, is cut on over the sun, a pink slash whips under it, then "v0.1" flickers on. The default theme's startup screen. |
| Spaceship | A purple galaxy in open space under a sci-fi overlay: scanner, spectrum, nav and target panels, a scan line tagging stars, a reticle locking on; then "MPFree OS" in the same blade letters, white to violet. |

The spiral galaxy is [`mpfree-splash`](../mpfree-splash).

Everything that never moves is drawn once into layers; a frame copies them
and draws only what moves. Scenes are deterministic: the same time gives the
same pixels.

## Preview and timing (workstation)

```bash
cargo run --manifest-path rust/Cargo.toml --release -p mpfree-startup -- --preview /tmp/startup-frames
cargo run --manifest-path rust/Cargo.toml --release -p mpfree-startup -- --bench
```

`--preview DIR` writes every frame as a PNG (new files only, DIR must be new
or empty), one folder per theme, or one theme with `--theme NAME`. `--bench`
prints the set-up time and the time per frame. Neither opens any device.

Fonts: the built-in Inter and JetBrains Mono from `mpfree-ui` (SIL OFL 1.1).

Fonts: VT323 (SIL OFL 1.1) is bundled unmodified in `fonts/` with its licence, for the console screen only. The Synthwave and Spaceship titles use blade letters drawn as polygons in `src/blade.rs` (no font file).

On the MPC One, `mpfree-startup --framebuffer` plays the default theme's screen on `/dev/fb0` and leaves its last frame showing. It writes only to the framebuffer and its sysfs `pan` file, never to storage.
