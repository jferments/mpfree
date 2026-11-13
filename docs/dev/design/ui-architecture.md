# UI architecture

**Status:** 2026-11-14 (the wheel's outline; on 13 Nov, controls listed for the smoke tests, and saying why in place); first written 2026-11-10. The toolkit is [`rust/mpfree-ui`](../../../rust/mpfree-ui); its first app is the Hello MPC pad player ([`gui.rs`](../../../rust/mpfree-app/src/gui.rs) draws it, [`app.rs`](../../../rust/mpfree-app/src/app.rs) decides what buttons and touches do). It first ran on the MPC in [round 4](../guides/mpfree-app.md#round-4-results-2026-11-10); round 5 starts it at power-on.

## What the owner asked for

From 2026-11-10 on, everything MPFree does gets a graphical interface on the MPC's screen, while the workstation's terminal keeps the full text log. The UI should be modular and composable, so that the layout and organisation can change completely as features and workflows are added. A debug mode shows MIDI, system events and timing.

The GUI decision of 2026-11-09 stands: SDL3 and a custom Rust retained-mode UI on OpenGL ES 3, with each finger belonging to the widget it first touched, and touch read through `mpfree-touch` rather than SDL. `mpfree-ui` is that custom UI. It draws in software for now, because a framebuffer is what the test build offers today; a GLES canvas can replace the software one later without changing any widget.

## The layers

From the bottom up:

| Layer | What it is | Where |
|---|---|---|
| Canvas | The only code that touches pixels: rectangles, rounded rectangles (anti-aliased), outlines, glyph masks, a clip rectangle. `SoftCanvas` draws into a plain XRGB8888 buffer. | [`canvas.rs`](../../../rust/mpfree-ui/src/canvas.rs) |
| Display list | `Recorder` is a canvas that keeps a frame's calls as a short list (`Op`s), with each mask and image stored once by content, and plays them back on any other canvas. Played back on a `SoftCanvas` the picture is the same bit for bit (a test checks all the sample screens). It is the seam for the GPU renderer: see "Drawing on the GPU" below. | [`record.rs`](../../../rust/mpfree-ui/src/record.rs) |
| Screen output | `FbTarget` copies only the changed rectangles to `/dev/fb0`, turned to the panel's mounting (the MPC One's panel is 800×1280, mounted turned, so the UI sees 1280×800). Turned 90 or 270 degrees, it reads the canvas in bands of 16 native rows, so the cache serves each band's pixels together (about 30 % less time for a whole frame on the workstation; the device number is still to come). It keeps a copy of what the panel shows, in the panel's layout, and turns only the changed rectangles into it, then writes whole native rows from that copy (turning whole rows from the canvas turned a whole screen column for every changed pixel column: the top bar, 1280 × 48, took 1.6 ms to turn whole and 0.06 ms as a rectangle, on the workstation). It also handles 16-bit framebuffers. It writes at least two whole native rows at a time: a write of exactly one row gives the kernel's framebuffer emulation a zero-width damage rectangle, and that row is never shown (found in round 4). `capture` reads the framebuffer back as a picture, which the loading page uses as its backdrop. | [`fb.rs`](../../../rust/mpfree-ui/src/fb.rs) |
| Theme | Every colour, size, radius and gap in one place, plus tones (normal, dim, accent, good, warning, bad) that widgets use instead of colours. Four built-in themes and user theme files; effects (corner shape, glow, backdrop, font roles) applied under the widgets by `Styled`. See [UI themes](ui-themes.md). | [`theme.rs`](../../../rust/mpfree-ui/src/theme.rs), [`style.rs`](../../../rust/mpfree-ui/src/style.rs) |
| Fonts | Three faces (sans, bold, mono), rasterised by `fontdue` and cached per size. Built in: Inter and JetBrains Mono (SIL OFL 1.1), cut down to Latin, about 4.5 MB in RAM once parsed. A copy of the fonts (`Fonts::clone`) shares the parsed faces and keeps its own glyph cache. Any TTF or OTF file can replace them (`--font`, `--bold-font`, `--mono-font`). | [`text.rs`](../../../rust/mpfree-ui/src/text.rs), [`fonts/`](../../../rust/mpfree-ui/fonts/README.md) |
| Widgets | Draw themselves into the rectangle they are given and turn touches into events. They never talk to hardware or the audio engine. | [`widgets.rs`](../../../rust/mpfree-ui/src/widgets.rs) |
| Layouts | A page is a one-line text spec naming widget slots in rows and columns. | [`layout.rs`](../../../rust/mpfree-ui/src/layout.rs) |
| Ui | Holds the widgets by name, the pages, the current page, what changed, and which widget owns each finger. Paints each pixel about once: a whole-page paint fills only the gaps between the widgets with the background, and each widget's part is cleared only where its first drawing (usually its opaque panel) will not cover it, its panel's corners (`Cleared`). The picture is the same as clearing everything first. | [`ui.rs`](../../../rust/mpfree-ui/src/ui.rs) |

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
- **NumPad** (`numpad.rs`): the number pad for typing a value (Jay after live test 3). A page asks with a `NumberAsk` (key, title, unit, value now, range, decimals); the pad keeps what is typed, `entered` checks the range (out of range, it sets a warning and the pad stays), and `nudge` steps it for the wheel and – / +. Every key is a `Tab(index)` (`KEY_ENTER`, `KEY_CANCEL` and the rest), each at least 64 px. A list row whose value can be typed is in `SettingList::typed`, drawn with a dotted line under its value, and a tap on that value gives `EventKind::Type(row)`. In the app the Sample tab's tabs answer it with `WaveAction::Ask`, the GUI opens the pad on the `Type a number` page over the page showing, and Enter goes back through the tab's `typed` as one undo step; a long press on Play's BPM opens it for the tempo (`Command::SetTempo`).
- **HistoryPanel** (`history.rs`): a page's undo history as a list (Jay after live test 3: hold UNDO to jump back several steps). The app sets its title, rows (Start, the steps undo takes back oldest first, then redo's) and the row that is now; a tap on a row is `Select(row)` and the app works out how many steps that is; `wheel` moves an outline and the app sends the wheel's push to the outlined row; a drag scrolls; Close is `Tab(TAB_CLOSE)`. Rows and Close are 64 px tall. In the app, UNDO held for 0.6 s (`ToUi::UndoButton`) opens it on the `Undo history` page over the page showing, for the history UNDO follows there (`Player::undo_source`): the loop, the takes and the kit's edits are the GUI's own and it refreshes them each frame; the Synth tab's and the mixer's turns are the audio thread's, asked for with `FromUi::UndoHistory`. While it or the number pad is open, `FromUi::Overlay` gives it the wheel, its push and – / + on every page.
- **OptionList:** rows of a name and a value with ‹ and › arrows (Prefs and each page's options). A tap on a row selects it; an arrow steps its value. While the data wheel's push has a row open (`editing`), its value box is outlined and its arrows lit in the accent colour, and the wheel steps the value instead of moving between rows. The Sample tab's `SettingList` and the Synth tab's `TileGrid` have the same `editing` flag (the value box outlined; the tile raised). Outside the menus the app sends the wheel, - / + and the push to the page showing as one `Effect::Rows(RowKey)`: `Pick` moves the highlight, `Step` steps the row picked, `Editing` opens or closes it; the GUI's `rows` hands each to the Sample, Synth or Arrange page (layout spec, section 6).

Feature crates add their own widgets on the same toolkit: [`mpfree-arp-ui`](arpeggiator.md) (the Arp panel, quick band and pattern workspace) and [`mpfree-time-ui`](playback.md#screens-mpfree-time-ui) (the transport bar, the Clips launch grid and the takes list).

Most pages keep a gap at the screen's edges; `add_full_page` makes one with none (the loading page).

### Events and state

Widgets show state and report what was touched; the app decides what that means. A touch becomes an `EventKind` (`Tab`, `Pad { index, down }`, `Select`, `Value`, `Step { index, delta }`, and others such as `Span { index, from, to }` for a swipe along a row) tagged with the widget's name. In the pad player, `app.rs` holds that meaning for buttons and touches alike (pages, menus, Note Repeat, Latch, Full Level, the button guide) and returns effects (show a page, light a button, change the sound); it never draws, so it is tested without a screen. The pad player turns those into the same actions the hardware produces: a tap on a screen pad and a hit on a real pad both become "play pad N"; a drag on a Q-Link card and a turn of its knob both become "turn Q-Link K". So any control can later come from the pads, the screen, an encoder or MIDI without the widgets knowing.

Each finger belongs to the widget it first touched until it lifts, so a finger that slides off the level slider keeps moving the slider rather than hitting a pad.

### Controls and the smoke tests

Every widget lists the controls it draws: `Widget::targets(&self, r)` returns a `Target` for each (default none), and `Ui::targets()` lists the page's, widget by widget in layout order, as `PageTarget { widget, target }`. Paint before listing: some widgets (the top bar's chips, the I/O settings) measure their controls when they draw.

A `Target` has an `id` that names the kind of control within its widget ("tab", "row-less", "knob-card"; the controls catalogue is keyed by `widget/id`), an `index` when there are several, the `label` the screen shows on it, the `rect` where it answers a finger, the `gesture` a finger makes on it (`Tap`, `Drag` or `Hold`), `towards` for a drag (which way the finger moves), and `enabled`. `enabled` is false (`disabled_if`) when the control is drawn faint, or is the one already lit (the tab shown, the row picked), so a press has nothing to change. The [GUI smoke tests](../guides/testing-and-ci.md#gui-smoke-tests) press every target the way its gesture says and fail on one that does nothing.

When a press can't do what it asks, the screen says so in place rather than doing nothing (Jay, 13 Nov 2026). A faint button or card still answers a tap, and the page puts why on the hint line (`Player::tell`): a faint looper tool, a blank Q-Link card, a faint take button or morph button on Arrange, a faint sampler button (`ButtonRow::why`) or Q-Link card (`KnobCard::why`), a sampler or Arrange setting already at its end (`time_view::why_not`, and `SamplePage` notices a step that changed nothing), an I/O setting that can't move (`IoSection::take_note`, read through the mixer's `take_io_note`). Controls that only show (status chips) say what they show.

The routing from the screen's actions to the engines (`FromUi` to the app, the mixer, the time engine, the looper and the chain editor) lives in `desk.rs`, shared by `main.rs` and the test `Session`, so the smoke tests drive the code the MPC runs.

### The wheel's outline

The data wheel moves an outline from control to control (Jay, 13 Nov; [layout spec](layout-spec.md), section 6). `Ui` keeps it, because only the widgets know their controls: each lists them with `Widget::targets` (the control audit's list: an id, an index, a label, a rectangle and a gesture, `Tap`, `Drag` or `Hold`).

- **Order:** `Ui` lists the page's controls panel by panel (by each slot's top, then its left) and each panel's in reading order: rows top to bottom, two controls sharing a row when either's middle is level with the other, each row left to right. Controls under another widget, or drawn as unavailable, are left out. `focus_move` steps through that list and stops at the ends; `focus_at` puts the outline on the smallest control under a finger, or takes it away.
- **Identity:** the outline remembers its widget and its target's `id` and `index`, not a rectangle, so it stays on the control while the widget changes round it, and goes with the page (`Ui::show`).
- **Turning a value:** a widget that the wheel can turn says so with `Widget::steps(t)`, and `Widget::step(t, delta, r)` changes the control by a small step per detent and returns the event a drag would (the mixer's faders, pans and sends by its Q-Link steps; the Q-Link cards as a drag of as many detents; the master level by 0.5 dB; an effect's knob by a hundredth of its travel; `Fader` likewise).
- **Drawing:** after the widgets, `Ui::paint` strokes the outline (3 px in `accent`, 5 px in `accent2` while grabbed) inside its widget's slot, whenever that widget was drawn. Moving the outline marks the old and new widgets to be drawn whole, which wipes the old line even on widgets that draw only their changed parts.
- **The pad player** (`gui.rs`): a finger coming down on an outlined page ([`app::outlined`](../../../rust/mpfree-app/src/app.rs)) moves the outline there. `Effect::Nav` carries the wheel (`Turn`), its push (`Push`) and – / + (`Step`) from `app.rs`; the push taps a `Tap` control with a finger of its own (`WHEEL_FINGER`), going through the same path as a real touch, holds a `Hold` control down until the next push, and grabs or lets go a value. The screen tells the app whether it shows an outline (`FromUi::Outline`), which decides the wheel on the mixer and the chain editor.

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

- **What is proven:** the MPC's Mali-T760 draws offscreen through Mesa's Panfrost driver (OpenGL ES 3.1; 2.4 ms a frame for an 800×480 test scene, [gpu-panfrost-offscreen.md](../milestones/gpu-panfrost-offscreen.md)). Live test 3 (13 Nov) ran the G2 device tool, panel step included (`mpfree-gpu-bench panel`: the GPU draws each page, it is read back and written to `/dev/fb0` as the app's screen writes): the GPU's way to the panel took 63 ms a frame against software's 32 ms, and a page's draw 28 ms against 11 ms.
- **Why it was slower, and the fix:** two costs. Reading the frame back and copying it to the framebuffer, which page flips remove: the GPU now draws straight into buffers the panel shows (`mpfree_gpu::kms` and `scanout`, two buffers shown in turn, in the panel's own portrait orientation, nothing read back or copied). And coverage maths for every pixel, even the inside of a panel: plain pixels now skip it (a rounded rectangle is three plain bands and four shaded corners, an outline four plain edges and four shaded corners), and drawing is cut to the rectangles the list changed. On the sample pages the shaded pixels per page fell from about 3 million to 50,000 to 100,000, with the same picture as the software canvas. The MPC's numbers for this way come from step 7 of the device tool (`flip`), which changes what the screen shows in RAM and waits for Jay's go.
- **The seam:** every screen draws through `Canvas`, so a frame recorded by `Recorder` is everything a renderer needs. The list for a whole screen is 8 to 490 KB on its first frame with an empty store (most of it glyphs and stroke masks seen for the first time) and 0 to 62 KB for the next frame (`every_sample_screen_plays_back_the_same_from_its_display_list` prints the table with `--nocapture`). The Sample and synth pages are the heaviest: their waveforms and curves are thousands of rectangles or large masks, which a GPU renderer would rather draw as lines.
- **The wire format:** `Recorder::encode_frame` turns a frame into bytes (little-endian; the store's dropped ids, the new masks and images with their ids, then the calls), `Frame::decode` reads it back, refusing anything short, oversized or unknown, and `Store::apply` keeps the receiver's copy of the store up to date. Played from the copy, every sample page is still the same picture bit for bit. It is how the app will feed a renderer process, and what `--display-lists` writes.
- **The renderer:** [`rust/mpfree-gpu`](../../../rust/mpfree-gpu/README.md) plays a list with OpenGL ES 2 straight into the panel's portrait orientation (the turn is part of the drawing; nothing is turned afterwards). On Mesa's software GPU its frame for a scene using every call matches the software path's bytes to within two levels per channel. Glyphs and other masks share one texture (an atlas), keyed by the store's mask numbers: every list a renderer plays must come from one store, and `Renderer::forget_all` must come first for lists from another (live test 3's garbled panel was the bench playing separately recorded pages, whose numbers start again at 0, on one renderer). One shader draws shapes and masks, so each of the 59 sample pages is a single draw call (672 at the median before the atlas); clips cost no call, as each quad is cut to the clip on whole pixels. Vector shapes (`Canvas::blend_shape` with an `mpfree_ui::Shape`: the synth and FX curves, knob rings and graphs, which were coverage masks until 14 Nov) go in the list as their parts; software rasterises them as before (every preview screen is the same, byte for byte), and the GPU works them out itself in a second atlas, so a synth page's first list is 46 KB instead of 488 KB and a curve that moves uploads nothing ([the GPU README](../../../rust/mpfree-gpu/README.md)). The wire format is version 2 for them, so the app and `mpfree-gpu-screen` must come from the same build: with a mismatched pair the process stops at the first frame (`fail: display list version ...`) and the app falls back to software, counted and shown as any fallback is. It is its own workspace, a dynamically linked program that loads `libEGL.so.1` at run time.
- **Its own process (G4):** with `mpfree app --gpu-screen PROGRAM` (or `play`), the screen thread paints each frame into a `Recorder` and sends the list down a pipe to `mpfree-gpu-screen`, which draws it into the panel's buffers and answers every frame (`mpfree_ui::gpu_screen`; [the GPU README](../../../rust/mpfree-gpu/README.md#the-screen-process-g4)). The app stays a static program and a crash or hang in the GPU's driver ends that process, never the sound. When the process can't start, fails, ends, or leaves a frame unanswered for 1.5 s, the app stops it (the screen goes back to the framebuffer), paints the whole screen in software, and stays in software until it starts again (`rust/mpfree-app/src/drawing.rs`). The fallback is counted, logged (stdout and the Events log) and shown: the Screen row reads "GPU · 30 fps · paint 1.2 · send 0.1 · GPU 3.4 ms" while the GPU draws (painting the list and sending it on the screen thread, then the GPU process's slowest frame) and "software, GPU off 1× · …" in the warning colour after a fallback ([profiling](profiling.md#the-screens-cost)). On quitting, the last frame is painted into the framebuffer too, so it stays up as with software drawing. A pipe rather than shared memory: after a page's first frame a list is 0 to 62 KB, and a pipe brings order and end of file for free. Off by default until a device round has run it.
- **Why not just copy less:** copying only the changed part of each native row would need writes shorter than a row, and the kernel's framebuffer emulation turns a one-row write into a zero-width update (round 4, above). The GPU path, with the panel reading the GPU's buffers directly, removes the copy instead.

## Next

- The GPU on the MPC: G2 ran in live test 3 (above). Next, on Jay's go: the page-flip step (`host_gpu_mpc.py --flip`, step 7: the panel showing the GPU's buffers, in RAM; see the [profiling design](profiling.md#gpu-benchmark-step-g2)), then the app with `--gpu-screen` (it needs `mpfree-gpu-screen` on the stick beside the app). A header chip saying the screen fell back to software would change the top bar, so it is a proposal for the layout spec's owner, not built.
- Gestures from `mpfree-touch` (swipe between pages, long press) and lists that scroll by drag.
- Widgets for what comes next: a step grid, a waveform, a mixer strip, a sample browser.
- Layouts and the theme read from files, so a layout can change without a rebuild. Parsing and applying both are done (`Theme::parse`, `Ui::apply_layouts`, see [UI themes](ui-themes.md)); a picker and a folder on the stick are next.
