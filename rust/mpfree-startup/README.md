# mpfree-startup

Themed startup screens for MPFree, drawn in software for the 1280x800 screen.
Each `mpfree-ui` theme names its startup screen; this crate draws three:

| Theme | Scene |
| --- | --- |
| Console | A chaotic swirl of green characters settles into "MPFree" and "OS v0.1" drawn in characters with the VT323 terminal font, shaded by density and colour; a prompt types out. Scanlines and bloom. |
| Synthwave | A striped sun rises over wavy neon hills and a racing grid; "MPFree" in Permanent Marker is slashed on over the sun, then "OS v0.1" flickers on. |
| Spaceship | The view from a ship's bridge: a purple galaxy fades up through the windshield while the dashboard boots (radar, boot log, spectrum, LEDs) and the HUD locks on; "MPFree" is wiped onto the glass. |

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

Fonts: VT323 (SIL OFL 1.1) and Permanent Marker (Apache 2.0) are bundled unmodified in `fonts/` with their licences, for the startup screens only.
