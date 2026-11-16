# UI themes and modular layouts

Status: first version in `mpfree-ui` (themes, effects, layout files) and the new `mpfree-startup` crate (themed startup screens). Asked for by the owner on 2026-11-11.

## What the owner asked for

- Users can make their own themes. The current look stays as the default.
- Three more built-in themes: **Console** (green on black, fluorescent glow, "Matrix" code, electric and pixelated), **Synthwave** (80s neon colours, glow and the geometric grid) and **Spaceship** (a dark purple sci-fi console: black, white and purple, in the spirit of the purple themes Ableton users like).
- A startup screen for each: a palm-tree sunset over a neon grid for Synthwave, green ASCII chaos that resolves into "MPFree OS" for Console, a galaxy for Spaceship.
- The UI in general should be modular, composable and easy to reorganise on a whim.

## Principles

1. **Widgets ask for meanings, not colours.** A widget draws `surface`, `accent` or `text_dim`, never `#6EA8FE`. A theme is the table that turns meanings into values.
2. **Skin and arrangement are separate.** A theme changes how things look; a layout changes where they are. Either can change without the other, and neither needs a rebuild.
3. **Effects live under the widgets.** Corner shape, glow, backdrop patterns and font swaps are applied by the drawing layer, so every screen in every crate gets them without code of its own.
4. **Files, not code, for users.** Themes and layouts are short text files with clear error messages that name the line.
5. **Cheap enough for the RK3288.** No effect needs a full-screen pass per frame. Anything that never moves is drawn once.

## How a theme is applied

```
widget ──draws meaning──▶ PaintCx ──▶ Styled (theme effects) ──▶ SoftCanvas ──▶ /dev/fb0
           (cx.theme.accent)              │
                                          ├─ fill in exactly `bg`  → the theme's backdrop
                                          ├─ rounded box          → round / cut / square corners
                                          ├─ outline              → + glow halo
                                          └─ text mask            → + soft text glow
Fonts: Sans / Bold roles → the theme's `face_body` / `face_title` (every built-in theme keeps the same UI font)
```

| Layer | What it holds | Code |
| --- | --- | --- |
| Tokens | Colours by meaning (`bg`, `surface`, `raised`, `outline`, `text`, `text_dim`, `text_faint`, `accent`, `accent2`, `on_accent`, `good`, `warn`, `bad`), eight `track` colours, sizes (`radius`, `gap`, `pad`, five text sizes). | [`theme.rs`](../../../rust/mpfree-ui/src/theme.rs) |
| Effects | `corners` (round, chamfer, square), `glow` (outline halo), `text_glow`, `backdrop` (solid, gradient, grid, dots, scanlines) with `backdrop_to`, `backdrop_line`, `backdrop_step`, and `tint` (how far kit and pad colours lean towards the accent on screen; pad lights are unchanged). | [`style.rs`](../../../rust/mpfree-ui/src/style.rs) |
| Faces | `face_body`, `face_title`: which of the three built-in faces (sans, bold, mono) the two text roles use. The owner wants one consistent UI font in every theme (2026-11-11), so the built-in themes all keep sans and bold; the hook stays for user themes. | [`text.rs`](../../../rust/mpfree-ui/src/text.rs) `Fonts::set_roles` |
| Startup | Which startup screen goes with the theme (`galaxy`, `console`, `synthwave`, `spaceship`). | [`mpfree-startup`](../../../rust/mpfree-startup) |

`Styled` draws the default theme pixel for pixel as before (a test checks this). The backdrop is laid out in screen coordinates, so a widget that redraws only part of itself lines up with the rest of the screen. `Ui::set_theme` switches theme at run time: tokens, effects and font roles change together and every page is laid out again with the new spacing.

## Theme files

A theme file is `key = value` lines. It starts from a built-in theme and lists only what it changes:

```text
# Sunrise: synthwave with an orange accent and cut corners
base          = synthwave
name          = Sunrise
accent        = #FF6C11
outline       = #FF6C11AA
backdrop      = dots
backdrop_line = #FF6C1166
corners       = chamfer
```

- Colours are `#RRGGBB` or `#RRGGBBAA` (a translucent `surface` lets the backdrop show through panels).
- `#` starts a comment when it begins the line or is followed by a space.
- Mistakes are errors that name the line: `theme line 3: accent: expected #RRGGBB, got "blue"`.
- `Theme::check()` warns when text is hard to read (WCAG contrast under 4.5:1 for text, 3:1 for dim text and accents), and when a button label's inks are under 4.5:1 on `raised`: both `text` and `text_dim`, because a label on a `raised` fill uses `Theme::button_ink(usable)` (text, or text_dim when the button can't be used) and never `text_faint` (Jay after live test 3: "light gray on a medium gray button is no bueno"). The built-in themes pass; a test keeps it that way.
- Every built-in theme is written out in full in [`rust/mpfree-ui/themes/`](../../../rust/mpfree-ui/themes) as a starting point, and `cargo run -p mpfree-ui --example theme_file -- NAME` prints one. A test keeps those files in step with the code.

The pad player's Prefs has one **Appearance** row whose value is the theme in use. Its arrows (or - / +, or SHIFT + wheel) step through Default, Console, Synthwave and Spaceship. Each step switches the screen at once (`Ui::set_theme`) and keeps it in the store's "app" settings as `theme`, so it comes back at the next start. `mpfree --theme NAME|FILE` still picks the theme for one run (for `--preview` and on the device) and wins over the saved one.

**Themes folder.** Theme files put in `files/themes/` on the data partition (`/run/mpfree-data/files/themes/` on the stick), named `NAME.theme` (lower case, digits, `.`, `_`, `-`), join the Appearance row after Spaceship, in name order, at most 32. The row shows the theme's `name` (or the file name when it has none or reuses a built-in theme's name). The app only reads the folder: a file that doesn't parse is left out with a log line (`themes: broken.theme left out: theme line 1: ...`), and `themes: found ...` lists the rest. Picking one keeps its file name as `theme`; if the file is gone at the next start, the default theme shows and the log says the saved theme was skipped.

## The built-in themes

| Theme | Look | Effects | Startup screen |
| --- | --- | --- | --- |
| Default | The current dark studio look, blue accent. | none | Synthwave's startup screen (Jay, 2026-11-11, from round 8). The app keeps the default look. The spiral galaxy (`mpfree-splash`) stays available. |
| Console | Phosphor green on near-black, in the normal UI font. | Cut corners, outline and text glow, scanline backdrop, kit colours tinted green. | A chaotic swirl of characters in many shades of green settles, arm by arm, into "MPFree OS" and "v0.1" in the same blade letters as the other screens, drawn as a grid of terminal characters (VT323), one per cell with none overlapping, each picked to match the shape and shading of its cell; then a prompt types out. Scanlines and bloom. |
| Synthwave | Hot pink and cyan on deep violet. | Strong neon glow on outlines, translucent panels over a grid. | A striped sun rises over rolling, wavy neon hills and a racing grid; "MPFree OS" in MPFree's own blade letters (angular, leaning strokes with torn, jagged starts and knife-point ends), yellow to pink over a hard drop shadow, is cut on over the sun and a pink slash whips under it; then "v0.1" flickers on below. No palm trees, no stripes in the letters. |
| Spaceship | Black, white and violet, thin lines. | Cut corners, soft glow, a faint dot grid. | A purple spiral galaxy in open space with a sci-fi computer overlay analysing it (no ship in view): heading tape, corner brackets, translucent panels (radar scanner, spectrum, navigation, target), a scan line that tags stars with callouts, a reticle that locks on the core; then "MPFree OS" in the same blade letters, white to violet, and "v0.1". |

## Startup screens and the CPU

One display font is bundled for the startup screens only, unmodified and with its licence in [`mpfree-startup/fonts`](../../../rust/mpfree-startup/fonts): VT323 (SIL OFL 1.1) for Console's character art. The Synthwave and Spaceship titles use MPFree's own blade letters, drawn as polygons in [`blade.rs`](../../../rust/mpfree-startup/src/blade.rs), so no font file or licence comes with them. The UI itself keeps its one font in every theme.

Each scene draws everything that never moves once (sky, mountains, palm trees, the galaxy's glow, text) and per frame only copies those layers and draws what moves. Time per 1280x800 frame on the workstation (one core, release build):

| Scene | Set-up | Frame (average) |
| --- | --- | --- |
| Console | 16 ms | 7.6 ms |
| Synthwave | 17 ms | 4.4 ms |
| Spaceship | 35 ms | 4.9 ms |

The RK3288 is several times slower per core; the estimate is 10 to 50 ms a frame, inside a 20 fps budget. `mpfree-startup --bench` measures it on the device (it only computes, no screen or storage access); that run is still to do. Scenes are deterministic (same time, same pixels), so previews and tests are exact.

The galaxy splash stays in `mpfree-splash`. The Spaceship galaxy is a different, cheaper particle galaxy under a HUD, so the two do not duplicate each other; if the splash thread wants one galaxy engine with a palette, Spaceship can switch to it.

`mpfree-startup --framebuffer [--theme NAME] [--rotation 270] [--seconds N]` plays a scene on `/dev/fb0` in real time and exits with its last frame showing, for the app's loading page (`app --keep-screen`). It writes only to the framebuffer device and its sysfs `pan` file, as `mpfree-splash` does, and never to storage. Without `--theme` it plays the default theme's startup screen (synthwave), or, with `--data DIR`, the startup screen of the theme picked in Prefs > Appearance (for a themes-folder file, its `startup` key or its base's). `--data` reads only `settings/app.conf` and that theme file, follows no symlinks and never writes; anything missing or odd shows synthwave.

With `--setup-status FILE` (or `MPFREE_SETUP_STATUS`, which the USB creator's sticks set in a drop-in for the splash unit), it does not exit when the scene ends while that file says a first-start format step is still working: it keeps the last frame up under the pad player's own loading card, with a bar that follows the bytes the step has written and the time left. [`setup.rs`](../../../rust/mpfree-startup/src/setup.rs) reads the file and smooths the bar; the [USB creator design](usb-creator.md#progress-the-musician-sees) describes the step. It only reads that file, and waits at most 330 s.

## Layouts: rearranging pages

The app's own layout (top bar, tab set and order, Play page, right-hand column, pad grid) is owned by the Hello MPC thread and set by its layout spec, which Jay approves (`/mnt/project-files/hello-mpc/layout-spec.md`). Themes never change it, and this PR ships no layout change. Layout files are a user's own rearrangement, read on top of the app's layouts.

Pages are already layout specs over named widgets (`col(header:60, row(pads:*, side:480):*, hint:28)`, see [UI architecture](ui-architecture.md)). New:

- `Ui::set_layout(page, spec)` rearranges one page; on an error the page keeps its old layout.
- `Ui::apply_layouts(text)` applies a layout file and checks every page before changing any:

```text
# Kit list on the left for left-handed playing
Play = col(header:60,
           row(side:480, pads:*):*,
           hint:28)
```

- `Ui::layout_of(page)` gives the current spec, so a future layout editor can show and save it.
- A file's layout for a page stays in force when the app adds that page again (widgets such as the arp panel re-add their page when their mode changes).
- Variants: a name with a dot (`Arp.pattern`) is a layout the app asks for by key with `Ui::layout_for(key, default)`, for pages whose layout changes with a mode.
- Shared values: `@side = 460` defines text that later lines use as `@side`, so pages that must line up (Play's side column and the arp pages that replace it, which keep the pads from jumping) share one number.

Next steps for composability, in order:

1. **Widget kinds in layout files.** Today a slot names a widget the app made. Next, a slot can make one: `knob(cutoff)`, `meter(master)`, `list(kits)`, bound to a parameter by name, so a user page can be built from parts without code.
2. **Containers:** `tabs(a, b, c)` and `stack(a, b)` (overlays, such as a popup over a page).
3. **Workspaces:** a set of pages and their layouts saved under a name, next to the theme.

## Retrofitting the other UI crates

What is automatic, for any crate that draws through `PaintCx`: colours that come from the theme, corner shapes, glow, backdrop, font roles. Screens were checked in all four themes with `mpfree --preview DIR --theme NAME`.

What each crate should change (small, mechanical):

1. **No hex colours in drawing code.** Use tokens; for a set of distinct colours (synth layers, tracks, graph series) use `t.track(i)`. `mpfree-synth-ui` is done: its panels build tiles and cards before they know the theme, so they name a colour by meaning (`QLINK_COLORS`, `TEXT`, `TEXT_DIM`, `TEXT_FAINT`, `ON_FILL`, the default theme's values) and `widgets::themed` swaps in the shown theme's token when painting (see [synths.md](synths.md)). That also covers `mpfree-fx-ui`, which uses the same widgets.
2. **Content colours through `t.content(c)`** (kit colours, pad colours, user-chosen track colours), so tinting themes can pull them in.
3. **Panels with `cx.panel(r)` / `cx.titled_panel(..)`**, buttons and pads with `fill_round_rect` at `t.radius` (or a little less): the corner style follows.
4. **`t.bg` means "background".** A fill in exactly `t.bg` becomes the theme's backdrop. To paint a solid colour that happens to look like the background, use `t.surface` or a token, not `t.bg`.
5. **Text colour on accent fills is `t.on_accent`.**
6. Previews: add `--theme` to the crate's preview binary and render every theme in tests (cheap, catches unreadable combinations).

## Later

- Startup screens in the boot path: the test stick runs `mpfree-startup --framebuffer` (from 12 Nov, in place of `mpfree-splash`), so it shows the default theme's startup screen, Synthwave's. It starts before the data partition is mounted, so it can't read the theme picked in Prefs yet; `--data /run/mpfree-data` is ready for when it can.
- With the GLES canvas, the same effects become shaders (bloom, scanlines, real blur) with no theme or widget changes.
- Optional per-theme sounds (UI clicks) and pad-light palettes.
