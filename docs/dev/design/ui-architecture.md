# UI architecture

**Status:** 2026-11-12 (the display list and the banded copy); first written 2026-11-10. The toolkit is [`rust/mpfree-ui`](../../../rust/mpfree-ui); its first app is the Hello MPC pad player ([`gui.rs`](../../../rust/mpfree-app/src/gui.rs) draws it, [`app.rs`](../../../rust/mpfree-app/src/app.rs) decides what buttons and touches do). It first ran on the MPC in [round 4](../guides/mpfree-app.md#round-4-results-2026-11-10); round 5 starts it at power-on.

## What the owner asked for

From 2026-11-10 on, everything MPFree does gets a graphical interface on the MPC's screen, while the workstation's terminal keeps the full text log. The UI should be modular and composable, so that the layout and organisation can change completely as features and workflows are added. A debug mode shows MIDI, system events and timing.

The GUI decision of 2026-11-09 stands: SDL3 and a custom Rust retained-mode UI on OpenGL ES 3, with each finger belonging to the widget it first touched, and touch read through `mpfree-touch` rather than SDL. `mpfree-ui` is that custom UI. It draws in software for now, because a framebuffer is what the test build offers today; a GLES canvas can replace the software one later without changing any widget.

## The layers

From the bottom up:

| Layer | What it is | Where |
|---|---|---|
| Canvas | The only code that touches pixels: rectangles, rounded rectangles (anti-aliased), outlines, glyph masks, a clip rectangle. `SoftCanvas` draws into a plain XRGB8888 buffer. | [`canvas.rs`](../../../rust/mpfree-ui/src/canvas.rs) |
| Display list | `Recorder` is a canvas that keeps a frame's calls as a short list (`Op`s), with each mask and image stored once by content, and plays them back on any other canvas. Played back on a `SoftCanvas` the picture is the same bit for bit (a test checks all the sample screens). It is the seam for the GPU renderer: see "Drawing on the GPU" below. | [`record.rs`](../../../rust/mpfree-ui/src/record.rs) |
| Screen output | `FbTarget` copies only the changed rectangles to `/dev/fb0`, turned to the panel's mounting (the MPC One's panel is 800×1280, mounted turned, so the UI sees 1280×800). Turned 90 or 270 degrees, it reads the canvas in bands of 16 native rows, so the cache serves each band's pixels together (about 30 % less time for a whole frame on the workstation; the device number is still to come). It also handles 16-bit framebuffers. It writes at least two whole native rows at a time: a write of exactly one row gives the kernel's framebuffer emulation a zero-width damage rectangle, and that row is never shown (found in round 4). `capture` reads the framebuffer back as a picture, which the loading page uses as its backdrop. | [`fb.rs`](../../../rust/mpfree-ui/src/fb.rs) |
| Theme | Every colour, size, radius and gap in one place, plus tones (normal, dim, accent, good, warning, bad) that widgets use instead of colours. Four built-in themes and user theme files; effects (corner shape, glow, backdrop, font roles) applied under the widgets by `Styled`. See [UI themes](ui-themes.md). | [`theme.rs`](../../../rust/mpfree-ui/src/theme.rs), [`style.rs`](../../../rust/mpfree-ui/src/style.rs) |
| Fonts | Three faces (sans, bold, mono), rasterised by `fontdue` and cached per size. Built in: Inter and JetBrains Mono (SIL OFL 1.1), cut down to Latin, about 1 MB in RAM. Any TTF or OTF file can replace them (`--font`, `--bold-font`, `--mono-font`). | [`text.rs`](../../../rust/mpfree-ui/src/text.rs), [`fonts/`](../../../rust/mpfree-ui/fonts/README.md) |
| Widgets | Draw themselves into the rectangle they are given and turn touches into events. They never talk to hardware or the audio engine. | [`widgets.rs`](../../../rust/mpfree-ui/src/widgets.rs) |
| Layouts | A page is a one-line text spec naming widget slots in rows and columns. | [`layout.rs`](../../../rust/mpfree-ui/src/layout.rs) |
| Ui | Holds the widgets by name, the pages, the current page, what changed, and which widget owns each finger. | [`ui.rs`](../../../rust/mpfree-ui/src/ui.rs) |

### Layouts

A page is written like this (the pad player's Play page):

```text
col(header:60, row(pads:*, col(kits:*, level:96, timing:206):460):*, hint:26)
```

`row(...)` places its children left to right and `col(...)` top to bottom, with the theme's gap between them. Any other name is a slot, filled by the widget added under that name. After a child, `:N` gives it N pixels, `:N*` a weighted share of what is left, and `:*` (or nothing) a share of weight 1. The same widget can sit on several pages: the timing panel is on both Play and Debug. Moving a panel, swapping two, or making a new page for a new workflow is a change to these strings, not to any widget.

### Widgets so far

- **Header:** the brand, tabs (one per page) and status chips on the right.
- **PadGrid:** the 16 pads as on the MPC (pad 1 bottom left), each with a colour, a label and a velocity bar; a hit brightens the pad and fades out. Only the pads that changed are redrawn.
- **ListView:** a titled list with one selected row, kept in view (the kits).
- **Fader:** a horizontal slider with a label and value (the level).
- **StatList:** label and value rows (timing).
- **LogPanel:** a scrolling log, each line with a tone (MIDI, events, the console).
- **Label:** one line of text (the hint at the bottom).
- **Loading:** a whole-screen start page: a backdrop (a captured picture, or its own stars and title), a card with what is happening, a detail line and a progress bar, or what failed. Only the card is redrawn as it changes.
- **Card:** a heading, lines of text and buttons (the button guide).
- **OptionList:** rows of a name and a value with ‹ and › arrows (Prefs and each page's options). A tap on a row selects it; an arrow steps its value.

Feature crates add their own widgets on the same toolkit: [`mpfree-arp-ui`](arpeggiator.md) (the Arp panel, quick band and pattern workspace) and [`mpfree-time-ui`](playback.md#screens-mpfree-time-ui) (the transport bar, the Clips launch grid and the takes list).

Most pages keep a gap at the screen's edges; `add_full_page` makes one with none (the loading page).

### Events and state

Widgets show state and report what was touched; the app decides what that means. A touch becomes an `EventKind` (`Tab`, `Pad { index, down }`, `Select`, `Value`, `Step { index, delta }`) tagged with the widget's name. In the pad player, `app.rs` holds that meaning for buttons and touches alike (pages, menus, Note Repeat, Latch, Full Level, the button guide) and returns effects (show a page, light a button, change the sound); it never draws, so it is tested without a screen. The pad player turns those into the same actions the hardware produces: a tap on a screen pad and a hit on a real pad both become "play pad N"; a tap on the kit list and a turn of the wheel both become "select kit K". So any control can later come from the pads, the screen, an encoder or MIDI without the widgets knowing.

Each finger belongs to the widget it first touched until it lifts, so a finger that slides off the level slider keeps moving the slider rather than hitting a pad.

### Threads and timing

In the pad player, the screen has its own thread at normal priority; the audio thread runs at `SCHED_FIFO` 70. They talk over two bounded channels:

- audio to screen: what happened (a pad hit, a kit change, a MIDI message, timing). The audio thread uses `try_send`, so it never waits for the screen; a full queue drops the message and counts it.
- screen to audio: actions from touches (play pad N, select kit K, set the level).

The screen thread redraws at most every 33 ms, and only what changed. It also prints every line to stdout, which reaches the workstation's terminal over SSH.

Measured on the workstation (x86, one core): a full 1280×800 paint takes about 2.5 ms, a pad hit's redraw about 0.05 ms, a new MIDI log line about 1.4 ms. The MPC's Cortex-A17 at 1.6 GHz will be slower; how much is not measured yet, and round 4 reports the screen's frame rate.

## Dependencies

`mpfree-ui` is the first MPFree crate with third-party code: the font rasteriser [`fontdue`](https://crates.io/crates/fontdue) 0.9.5 (MIT, Apache-2.0 or Zlib), and through it `ttf-parser` (MIT or Apache-2.0), `core_maths` and `libm` (MIT). All are permissive and compatible with MPFree's MIT licence; `Cargo.lock` pins them and every build uses `--locked`.

`mpfree-ui` itself forbids unsafe code. `fontdue` has a little: unchecked indexing in its rasteriser and character decoding, guarded by its own checks of the font. With the built-in fonts that is well-trodden code; a font file given with `--font` is parsed by the same code, so only use font files you trust.

## Testing without the MPC

- Unit tests cover layouts, drawing (pixels inside their rectangles, anti-aliased corners), text fitting, the framebuffer's rotation, widget touches and partial redraws. CI runs them on the workstation and as 32-bit ARM under qemu.
- `mpfree --preview DIR` draws every screen with sample data into PNG files. CI runs it for ARM under qemu. When checked on 2026-11-10, the ARM pictures matched the workstation's pixel for pixel.
- `mpfree --display-lists DIR` records every sample page's first two frames (the page as first drawn, then what it redraws a tick later) as display lists, one `NAME.mpdl` file per page, for `mpfree-gpu-bench`.

## Drawing on the GPU

Jay, 12 Nov: "we should ABSOLUTELY be drawing on the GPU", drawn in the panel's own orientation instead of turned afterwards. The plan is the "GPU drawing and compute" tab of the performance plan (https://claude.ai/artifact/Su4KF6BVbzkm9d5NfnWQMT); in short:

- **What is proven:** the MPC's Mali-T760 draws offscreen through Mesa's Panfrost driver (OpenGL ES 3.1; 2.4 ms a frame for an 800×480 test scene, [gpu-panfrost-offscreen.md](../milestones/gpu-panfrost-offscreen.md)). Nothing from the GPU has been shown on the panel yet.
- **The seam:** every screen draws through `Canvas`, so a frame recorded by `Recorder` is everything a renderer needs. The list for a whole screen is 8 to 490 KB on its first frame with an empty store (most of it glyphs and stroke masks seen for the first time) and 0 to 62 KB for the next frame (`every_sample_screen_plays_back_the_same_from_its_display_list` prints the table with `--nocapture`). The Sample and synth pages are the heaviest: their waveforms and curves are thousands of rectangles or large masks, which a GPU renderer would rather draw as lines.
- **The wire format:** `Recorder::encode_frame` turns a frame into bytes (little-endian; the store's dropped ids, the new masks and images with their ids, then the calls), `Frame::decode` reads it back, refusing anything short, oversized or unknown, and `Store::apply` keeps the receiver's copy of the store up to date. Played from the copy, every sample page is still the same picture bit for bit. It is how the app will feed a renderer process, and what `--display-lists` writes.
- **The renderer:** [`rust/mpfree-gpu`](../../../rust/mpfree-gpu/README.md) plays a list with OpenGL ES 2 straight into the panel's portrait orientation (the turn is part of the drawing; nothing is turned afterwards). On Mesa's software GPU its frame for a scene using all six calls matches the software path's bytes to within two levels per channel. Glyphs and other masks share one texture (an atlas), and one shader draws shapes and masks, so each of the 59 sample pages is a single draw call (672 at the median before the atlas); clips cost no call, as each quad is cut to the clip on whole pixels. It is its own workspace, a dynamically linked program that loads `libEGL.so.1` at run time.
- **Recommended next:** run that renderer as a small process of its own on the MPC, fed the list through shared memory. The app stays a static program, a crash in the GPU driver cannot stop the sound, and if the GPU process fails the app draws in software (counted and shown).
- **Why not just copy less:** copying only the changed part of each native row would need writes shorter than a row, and the kernel's framebuffer emulation turns a one-row write into a zero-width update (round 4, above). The GPU path, with the panel reading the GPU's buffers directly, removes the copy instead.

## Next

- The GPU renderer on the MPC: a full-size offscreen timing run (step G2: `mpfree-gpu-bench` through `tools/mpc-one/host_gpu_mpc.py`, every sample page on the GPU and in software, plus a compute round trip; see the [profiling design](profiling.md#gpu-benchmark-step-g2)), then its own process fed by the app (G4); the read-only GPU inventory is in the app thread's next round. The panel showing the GPU's buffers (G5) changes the display settings in RAM and waits for Jay's go.
- Gestures from `mpfree-touch` (swipe between pages, long press) and lists that scroll by drag.
- Widgets for what comes next: a step grid, a waveform, a mixer strip, a sample browser.
- Layouts and the theme read from files, so a layout can change without a rebuild. Parsing and applying both are done (`Theme::parse`, `Ui::apply_layouts`, see [UI themes](ui-themes.md)); a picker and a folder on the stick are next.
