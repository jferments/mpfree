# mpfree-startup

Themed startup screens for MPFree, drawn in software for the 1280x800 screen.
Each `mpfree-ui` theme names its startup screen; this crate draws three:

| Theme | Scene |
| --- | --- |
| Console | Green ASCII rain and noise resolve, cell by cell, into "MPFree" in ASCII art; a short log types out. Half resolution doubled, with scanlines and bloom. |
| Synthwave | A striped sun rises behind neon mountains and palm trees over a racing grid; a chrome "MPFree" flickers on like a neon tube. |
| Spaceship | The ship drops out of warp in front of a purple spiral galaxy; the HUD boots and the title is wiped on by a scan line. |

The default theme's galaxy is [`mpfree-splash`](../mpfree-splash).

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
