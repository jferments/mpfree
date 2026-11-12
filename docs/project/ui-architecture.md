# UI architecture

**Status:** 2026-11-10. The toolkit is [`rust/mpfree-ui`](../../rust/mpfree-ui); its first app is the Hello MPC pad player ([`gui.rs`](../../rust/mpfree-app/src/gui.rs) draws it, [`app.rs`](../../rust/mpfree-app/src/app.rs) decides what buttons and touches do). It first ran on the MPC in [round 4](../guides/mpfree-app.md#round-4-results-2026-11-10); round 5 starts it at power-on.

## What the owner asked for

From 2026-11-10 on, everything MPFree does gets a graphical interface on the MPC's screen, while the workstation's terminal keeps the full text log. The UI should be modular and composable, so that the layout and organisation can change completely as features and workflows are added. A debug mode shows MIDI, system events and timing.

The GUI decision of 2026-11-09 stands: SDL3 and a custom Rust retained-mode UI on OpenGL ES 3, with each finger belonging to the widget it first touched, and touch read through `mpfree-touch` rather than SDL. `mpfree-ui` is that custom UI. It draws in software for now, because a framebuffer is what the test build offers today; a GLES canvas can replace the software one later without changing any widget.

## The layers

From the bottom up:

| Layer | What it is | Where |
|---|---|---|
| Canvas | The only code that touches pixels: rectangles, rounded rectangles (anti-aliased), outlines, glyph masks, a clip rectangle. `SoftCanvas` draws into a plain XRGB8888 buffer. | [`canvas.rs`](../../rust/mpfree-ui/src/canvas.rs) |
| Screen output | `FbTarget` copies only the changed rectangles to `/dev/fb0`, turned to the panel's mounting (the MPC One's panel is 800×1280, mounted turned, so the UI sees 1280×800). It also handles 16-bit framebuffers. It writes at least two whole native rows at a time: a write of exactly one row gives the kernel's framebuffer emulation a zero-width damage rectangle, and that row is never shown (found in round 4). `capture` reads the framebuffer back as a picture, which the loading page uses as its backdrop. | [`fb.rs`](../../rust/mpfree-ui/src/fb.rs) |
| Theme | Every colour, size, radius and gap in one place, plus tones (normal, dim, accent, good, warning, bad) that widgets use instead of colours. Four built-in themes and user theme files; effects (corner shape, glow, backdrop, font roles) applied under the widgets by `Styled`. See [UI themes](ui-themes.md). | [`theme.rs`](../../rust/mpfree-ui/src/theme.rs), [`style.rs`](../../rust/mpfree-ui/src/style.rs) |
| Fonts | Three faces (sans, bold, mono), rasterised by `fontdue` and cached per size. Built in: Inter and JetBrains Mono (SIL OFL 1.1), cut down to Latin, about 1 MB in RAM. Any TTF or OTF file can replace them (`--font`, `--bold-font`, `--mono-font`). | [`text.rs`](../../rust/mpfree-ui/src/text.rs), [`fonts/`](../../rust/mpfree-ui/fonts/README.md) |
| Widgets | Draw themselves into the rectangle they are given and turn touches into events. They never talk to hardware or the audio engine. | [`widgets.rs`](../../rust/mpfree-ui/src/widgets.rs) |
| Layouts | A page is a one-line text spec naming widget slots in rows and columns. | [`layout.rs`](../../rust/mpfree-ui/src/layout.rs) |
| Ui | Holds the widgets by name, the pages, the current page, what changed, and which widget owns each finger. | [`ui.rs`](../../rust/mpfree-ui/src/ui.rs) |

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

## Next

- A GLES canvas behind the same `Canvas` trait, once a GPU path is qualified on the test build.
- Gestures from `mpfree-touch` (swipe between pages, long press) and lists that scroll by drag.
- Widgets for what comes next: a step grid, a waveform, a mixer strip, a sample browser.
- Layouts and the theme read from files, so a layout can change without a rebuild. Parsing and applying both are done (`Theme::parse`, `Ui::apply_layouts`, see [UI themes](ui-themes.md)); a picker and a folder on the stick are next.
