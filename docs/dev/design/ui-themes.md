# UI themes and modular layouts

Status: first version in `mpfree-ui` (themes, effects, layout files) and the new `mpfree-startup` crate (themed startup screens). Asked for by the owner on 2026-11-11. The theme refactor (colour roles, theme files that follow and mix colours, the contrast and fixed-colour checks, then a Light theme) started on 2026-11-22: see [The theme refactor](#the-theme-refactor) for where it stands.

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
Sizes and fonts: the layout's (sizes.rs), the same in every theme
```

| Layer | What it holds | Code |
| --- | --- | --- |
| Tokens | Core colours by meaning (`bg`, `surface`, `raised`, `outline`, `text`, `text_dim`, `text_faint`, `accent`, `accent2`, `on_accent`, `good`, `warn`, `bad`), eight `track` colours, the corner `radius`. | [`theme.rs`](../../../rust/mpfree-ui/src/theme.rs) |
| Roles | Every other colour a screen paints, each with a default rule in the core colours (`qlink.1 = {track.1}`), and the helpers screens use: `ink_on`, `swatch`, `round`, `Paint`. See [Colour roles](#colour-roles). | [`roles.rs`](../../../rust/mpfree-ui/src/roles.rs) |
| Effects | `corners` (round, chamfer, square), `glow` (outline halo), `text_glow`, `backdrop` (solid, gradient, grid, dots, scanlines) with `backdrop_to`, `backdrop_line`, `backdrop_step`, and `tint` (how far kit and pad colours lean towards the accent on screen; pad lights are unchanged). | [`style.rs`](../../../rust/mpfree-ui/src/style.rs) |
| Sizes and faces | Not a theme's: the five text sizes, `gap`, `pad` and the two faces are the layout's, the same in every theme (owner, 22 Nov: no theme may break a layout; one UI font since 11 Nov). A theme still carries them (`t.size_body`, `t.pad`) so drawing code reads every measure in one place, always these. A theme file that sets one (as files from before 22 Nov did) still loads; each such line is ignored with a note (`ProblemKind::Ignored`). | [`sizes.rs`](../../../rust/mpfree-ui/src/sizes.rs) |
| Startup | Which startup screen goes with the theme (`galaxy`, `console`, `synthwave`, `spaceship`). | [`mpfree-startup`](../../../rust/mpfree-startup) |

`Styled` draws the default theme pixel for pixel as before (a test checks this). The backdrop is laid out in screen coordinates, so a widget that redraws only part of itself lines up with the rest of the screen. `Ui::set_theme` switches theme at run time: colours, corners and effects change together; sizes and spacing never do, so every page keeps its layout and every touch target stays where it was.

## Colour roles

A theme sets the core colours. Everything else a screen paints is a **role**: a named colour with a default rule written in the core colours, so a theme that sets only the core colours still gets every role, turned the right way for light or dark. A theme file can set any role to a colour or a rule of its own.

The roles are one table, `ROLES` in [`roles.rs`](../../../rust/mpfree-ui/src/roles.rs): key, group, tier (core or role), default rule and one line of what it colours. The theme file's keys, `Theme::to_text`, the readability check and (in phase 4) the manual's role list all come from it. **A role is added in the same pull request as the first drawing code that uses it**, never ahead of it, so the table only holds roles something draws.

| Key | Group | Default rule | What it colours |
| --- | --- | --- | --- |
| `record` | Status | `{bad}` | Recording: the record button, armed inputs, the level while recording |
| `qlink.1` to `qlink.4` | Controls | `{track.1}` to `{track.4}` | The four Q-Link knobs: rings, Q1 to Q4 labels, what each moves |
| `text_inverse` | Text | `light_dark({surface}, {bg})` | Text on a fill too bright (or too dark) for `text`; a light theme's is its palest colour |
| `highlight` | Light and shade | `{text}` | Bright marks: what a pad or button flashes towards when hit, the ring round a picked cell |
| `shadow` | Light and shade | `light_dark({text}, {bg})` | Drop shadows (drawn see-through) |
| `series.1` to `series.8` | Palettes | `{track.1}` to `{track.8}` | Graph lines and EQ bands, in order |
| `handle` | Controls | `light_dark({surface}, {text})` | A switch's knob, a slider's handle |
| `cut` | Palettes | `{text_dim}` | The EQ's low and high cuts, beside its bands' series colours |
| `grain` | Sampler | `{accent2}` | The grain window over the wave and its GRAIN and SIZE marks |

Every built-in theme sets some roles to the exact colours its screens drew before roles existed (the sampler's record red `#FF4D5E` and grain purple `#C77DFF`, the EQ's band and cut colours, the Play page's white flash and black shadow, the white of switch knobs and slider handles, and outside Default the default Q-Link colours), so moving a screen onto roles changes no pixel. Light (phase 4) uses the rules. A theme file with `base = default` copies those colours too, like everything else the base sets; a line such as `record = {bad}` puts a role back on its rule.

What screens call:

- `t.role(Role::X)`, or the helpers `t.qlink(i)`, `t.series(i)`, `t.record()`; `t.color(key)` reads any colour by its key.
- `t.ink_on(fill)`: the label colour for any coloured fill, `text` or `text_inverse`, whichever reads better on it. It replaces `t.bg` used as dark text, and `on_accent` anywhere but on the accent.
- `t.swatch(c)` for a content colour (a kit's, a pad's, a track's own): `fill` (as the theme shows it, through `t.content`), `tint` (18 % of it over the panel, for big areas), `line` (moved towards `text` just far enough for 3:1 on panels), `text` (the same for 4.5:1) and `ink` (`ink_on(fill)`). A colour that already reads comes back unchanged, which is why Default's pastels keep their pixels.
- `t.readable(ink, fill)`: text in a colour of its own (a Q-Link knob's number, a macro's) on a fill the theme doesn't choose, moved towards whichever of `text` and `text_inverse` reads on the fill just far enough for 4.5:1. Text that already reads comes back unchanged.
- `t.faded(ink, fill)`: the text of a control that does nothing as set, half see-through, but drawn just solid enough to keep 3:1 on `fill` where half would read less (a dark ink on a light panel). In the dark built-in themes it comes back half, as before.
- `t.round(r)`: a corner radius drawn for Default's 14 px radius, scaled to the theme's, so one number, `radius`, sets every corner. Default gets `r` back. Circles, dots and pills keep their own radius.
- `t.keys()`: a piano keyboard's white, black and marked keys, the right way round on a light or dark theme (white keys a quarter of the way from `text` to `bg` on a dark one, `surface` on a light one). Label a key with `ink_on(key)`.
- `Paint` (a colour key, a colour key see-through, two keys mixed, a content colour, or a literal for artwork) for widgets built before they know the theme; `paint.on(&theme)` resolves it when drawn, so a page shown before the theme changed still draws in the new one. `Paint::ACCENT`, `Paint::TEXT_DIM` and the rest name the core colours, `Paint::track(i)` a track colour, and `paint.with_alpha(a)` makes one see-through. The sampler's knob cards, pad bar, wave marks, lane cells and grain window keep one, and so do `mpfree-synth-ui`'s tiles, cards, curves and boxes (the FX tab's too): its `themed()`, which matched colours by value, is gone.
- `t.is_light()`: the theme's `mode`, or on `auto`, whether `bg` is brighter than `text`.

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

- Colours are `#RRGGBB` or `#RRGGBBAA` (a translucent `surface` lets the backdrop show through panels), or follow and mix others:
  - `{accent}` (or just `accent`) is another colour of the theme: a core colour, `track.1` to `track.8`, `backdrop_to`, `backdrop_line` or a role;
  - `mix(a, b, 30%)` is 30 % of the way from `a` to `b`; `alpha(a, 40%)` is `a` at 40 % opacity;
  - `light_dark(a, b)` is `a` on a light theme and `b` on a dark one, like CSS's `light-dark()`. `mode = auto | light | dark` says which the theme is (`auto` compares `bg` and `text`).
  A colour may follow one set further down the file. A circle (`accent = {accent2}`, `accent2 = {accent}`) is a mistake that names both lines; both keep the base theme's colours.
- `track = a b c` sets the eight track colours (a shorter list repeats); `track.3 = ...` sets one, and wins over the list.
- `#` starts a comment when it begins the line or is followed by a space.
- Two readings. `Theme::load` never fails: a line with a mistake leaves what it would have changed as the base has it, an unknown key (from a newer MPFree, say) is skipped, and each is listed as a `Problem` (line, kind, what is wrong, what was used instead), followed by whether the theme is unusable and the readability warnings. The app reads the themes folder this way (phase 5, the owner's pick on 22 Nov 07:17Z), and Prefs > Appearance lists each file's problems, so no theme falls back without saying so. `Theme::parse` is strict, for `--theme FILE` and tests: any mistake or unknown key is an error that names the line (`theme line 3: accent: expected a colour (#RRGGBB, {name}, mix(), alpha() or light_dark()), got "blue"`).
- `Theme::check()` warns when text is hard to read (WCAG contrast under 4.5:1 for text, 3:1 for dim text, accents and the roles drawn as lines and labels: `record`, `qlink.N`, `series.N`), and when a button label's inks are under 4.5:1 on `raised`: both `text` and `text_dim`, because a label on a `raised` fill uses `Theme::button_ink(usable)` (text, or text_dim when the button can't be used) and never `text_faint` (Jay after live test 3: "light gray on a medium gray button is no bueno"). The built-in themes pass; a test keeps it that way.
- `Theme::unusable()` is the floor under those warnings: `text` or `text_dim` under 3:1 (`FLOOR`, the line the contrast check calls unreadable) on `bg`, on `surface` as it shows over `bg`, or on `raised` over that. Below it nothing could be read, Prefs > Appearance included, so no one could pick another theme. `Theme::load` lists it as a `ProblemKind::Unusable` problem and `Theme::parse` refuses the file (`nothing could be read: text on panels (surface) 1.0:1, under the 3:1 every theme must reach`). A theme between the floor and the warnings still loads. Every built-in theme is far above it (a test keeps it so; Default's lowest is `text_dim` on `raised` at 5.7:1).
- `mpfree --check-theme FILE` is the theme writer's check, on any computer that builds MPFree (no MPC, no devices opened; about 20 s): it lists the file's problems by line (`Theme::load`), says whether the app would offer it in Prefs > Appearance or leave it out (`Theme::parse`, the floor included), then draws every sample screen in the theme and in the theme its `base` names and lists each page with more text under 3:1 than the base has, every run with its ink, what is under it, its place and its contrast (the contrast check's measure, `mpfree_ui::lint`). It ends with a STOP line and exit status 1 when the app would leave the file out. The report comes last, after the lines the sample screens print as they are built:

  ```text
  theme Pale (pale.theme)
    unusable: nothing could be read: text_dim on the page (bg) 2.5:1, ...
  the app leaves this file out of Prefs > Appearance: nothing could be read: ...
  play: 45 run(s) of text under 3:1 (light: 0)
    #8A93A3 on #E4E7EB at 104,34 (40 glyph(s), 2.5:1)
  ...
  screens: 171 drawn, 5216 run(s) of text under 3:1 (light: 59), 171 page(s) with more than light
  STOP: pale.theme: nothing could be read: ...
  ```
- `mpfree --contact-sheet DIR --theme NAME|FILE` draws every sample screen in the theme and in the theme it starts from (a file's `base`, else Default; Light beside Default itself) as new PNG files in `DIR/theme/` and `DIR/base/`, and writes `DIR/index.html`, one page with no scripts that shows each screen's pair side by side and opens from disk. It is what a theme writer looks over before sharing a theme (Light's approval sheet was made the same way). About 30 s.
- Every built-in theme is written out in full in [`rust/mpfree-ui/themes/`](../../../rust/mpfree-ui/themes) as a starting point, roles included (as their rules, such as `qlink.2 = {track.2}`), and `cargo run -p mpfree-ui --example theme_file -- NAME` prints one. A test keeps those files in step with the code. Three commented examples show how to start: [`example-sunrise.theme`](../../../rust/mpfree-ui/themes/example-sunrise.theme) (Synthwave with an orange accent: following and fading colours), [`example-paper.theme`](../../../rust/mpfree-ui/themes/example-paper.theme) (a warm light theme on Light) and [`example-midnight.theme`](../../../rust/mpfree-ui/themes/example-midnight.theme) (a dark one on Default with an amber accent and dark words on it). A test keeps each readable, and `--check-theme` finds no screen with more unreadable text than its base.

The pad player's Prefs has one **Appearance** row whose value is the theme in use. Its arrows (or - / +, or SHIFT + wheel) step through Default, Slate, Light, Console, Synthwave and Spaceship. Each step switches the screen at once (`Ui::set_theme`) and keeps it in the store's "app" settings as `theme`, so it comes back at the next start. `mpfree --theme NAME|FILE` still picks the theme for one run (for `--preview` and on the device) and wins over the saved one.

**Themes folder.** Theme files put in `files/themes/` on the data partition (`/run/mpfree-data/files/themes/` on the stick), named `NAME.theme` (lower case, digits, `.`, `_`, `-`), join the Appearance row after Spaceship, in name order, at most 32. The row shows the theme's `name` (or the file name when it has none or reuses a built-in theme's name). The app only reads the folder, each file with `Theme::load`. A line with a mistake, or a key it doesn't know, uses the base theme's value and is logged (`themes: sunrise.theme: used the base for: line 3: accent: ...`), as are ignored lines and colours hard to read; Prefs > Appearance lists them while that theme is in use. Only a file whose text no one could read (`Theme::unusable`), or that isn't text, is left out, with a log line (`themes: fog.theme left out: nothing could be read: ...`), and the Appearance panel names it; `themes: found ...` lists the rest. Picking one keeps its file name as `theme`; if the file is gone or left out at the next start, the default theme shows and the log says so (`theme: the saved theme fog.theme can't be used now ..., so the default theme shows`). A theme file given with `--theme` that is refused shows the default theme too (`...; using the default theme`). These are fallbacks: the Dev tab's CPU by engine page counts the files left out, the theme file lines that used the base's value, and the starts with the default theme in place of the one asked for; the startup screen follows the same rules (`mpfree-startup` reads the file the same way).

**Keep this theme?** (layout spec section 5, the owner's pick on 22 Nov 07:17Z). A change on the Appearance row starts a trial (`App::theme_trial`): the screen switches at once, and Prefs' panel asks "Keep Console?" with "Back to Default in 15 s" counting down (`App::theme_tick`, from the screen loop after `App::tick`) and two buttons, Keep and Go back (`FromUi::HelpButton`). Keep, or the wheel's push on Appearance (also the push that sets an open Appearance row), keeps it; Go back, leaving Prefs, or the time running out puts the theme from before back, with a log line (`theme: kept Console`, `theme: back to Default (Console not kept)`). Another step restarts the 15 s and still goes back to the theme from before the first step; stepping back to that theme ends the trial. Until it is kept, the saved settings hold the old theme (`App::kept_theme`), so a theme no one can read past is never what the next start opens with.

## Checks

Two tests keep every theme usable as screens change. Both run in the normal `cargo test`, and both are ratchets: a count may go down at any time (the theme work lowers them as it goes) but going up fails.

**Contrast on every screen, in every built-in theme** ([`mpfree-app/tests/theme_contrast.rs`](../../../rust/mpfree-app/tests/theme_contrast.rs)). `mpfree --display-lists DIR --theme all` draws each sample page in each built-in theme (the screens are built once). [`mpfree_ui::lint::text_runs`](../../../rust/mpfree-ui/src/lint.rs) plays each page's display list back and measures every run of text (glyphs of one ink on one line) against the average of the pixels under it, before it is drawn; a glowing theme's text glow is not counted as text. A run under 3:1 is unreadable. [`golden/theme-contrast.txt`](../../../rust/mpfree-app/golden/theme-contrast.txt) lists how many each theme's page may have, each line with its reason. On 22 Nov (phase 0) that was 553 runs in Default, 355 in Console, 319 in Synthwave and 2,594 in Spaceship; most are `text_faint` on `raised` or on a panel (2.6:1 in Default, 2.8:1 in Spaceship), and over 2,000 of Spaceship's are the arp panel's faint step marks. A page with more fails with the new runs' colours and places. If the new run is meant, write the list (`MPFREE_LINT_BLESS=1 cargo test -p mpfree-app --test theme_contrast`) and give its line a reason. About 25 s on the workstation.

**No new fixed colours in drawing code** ([`mpfree-ui/tests/fixed_colours.rs`](../../../rust/mpfree-ui/tests/fixed_colours.rs)). It counts hex numbers of six or eight digits that aren't one side of a bit operation, and `Color::rgb(0)`, in the code of every crate that draws (`mpfree-ui`, `mpfree-app`, `mpfree-mpcskin`, every `mpfree-*-ui`), comments, strings and `#[cfg(test)]` items left out, and holds each file to its count in [`tests/fixed-colours.txt`](../../../rust/mpfree-ui/tests/fixed-colours.txt). Files where hex numbers aren't colours, or where colours are meant to be written down (the built-in themes), are listed with `*`. A file with more fails, naming the lines: use a core colour, a role, or `theme.swatch(colour)` instead, or add a role (in `roles.rs`, with its rule) if nothing fits. `MPFREE_LINT_BLESS=1 cargo test -p mpfree-ui --test fixed_colours` writes the counts.

`Theme::check` covers each theme's own pairs (text on panels and buttons, accents, the line roles), and the theme file tests keep the written-out built-in themes in step.

## The theme refactor

Asked for on 2026-11-22 (a Reddit request for a light theme; the owner's go the same day): move every screen onto theme roles so themes are simple to write, then ship a **Light** theme with the default dark one as the proof. Themes change colours and styles, never layout, the font or the pad lights. Six phases, Default pixel for pixel the same through phases 0 to 3 (the golden screens prove it):

| Phase | What | State |
| --- | --- | --- |
| 0 | Roles table and helpers, theme files that follow and mix colours, `Theme::load` with problems, the contrast and fixed-colour checks | #762 |
| 1 | Shared widgets (`mpfree-ui`, the app's frame): Q-Link colours as `qlink` roles (`draw::KNOB` is gone; the Q-Link column's numbers through `t.readable`), labels on coloured fills (pills, wave markers, pads) through `ink_on`, card corners through `t.round`. Default draws the same display lists; in the other built-in themes the shared widgets' corners follow `radius`. A trial light palette's unreadable runs fall from 1,063 to about 400, the rest in feature crates | #772 |
| 2 | One pull request per feature crate (sampler, chain and mixer EQ, time, synth and FX with `Paint`, MIDI, arp and browser, the app's Play, Perform and Looper, and `mpfree-mpcskin`) | #772: sampler (its two palettes one, record red and grain purple from roles, knob cards, marks and lane cells keep a `Paint`, labels on fills through `ink_on`, keyboards through `t.keys()`; every Sample page reads in the trial light palette); chain and mixer EQ (bands from `series`, cuts from `cut`, dot and button labels through `ink_on`, the notes keyboard through `t.keys()`, corners through `t.round`; every chain and mixer page reads in the trial light palette); MIDI, arp, browser and the FX pads (card corners through `t.round`, labels on warn and accent2 fills through `ink_on`, on accent fills `on_accent`, the MIDI monitor's keyboard through `t.keys()`; their pages already read in the trial light palette); synth and FX (tiles, cards, curves and boxes keep a `Paint`, so `themed()` and its colour matching are gone and the effect cards no longer need the theme to be built; a section shades a drawing's accent, good, warn and second accent like Q-Link colours 1 to 4, as the default theme did by value; labels on job and Q-Link fills through `ink_on`, a switched-off control's labels through `t.faded`, corners through `t.round`; every Synth page reads in the trial light palette); `mpfree-mpcskin` (the pop-ups over skins follow the theme with the guards below, the two pop-up faults fixed); time (clips, Arrange and the song lanes) (track names on their own colour, the clips' second lines and words on cells through `t.readable`, labels on the transport's lit buttons, scene buttons, Rec song and the song tags through `ink_on`, corners through `t.round`, the note editor's fallback colour the third track colour; the Default clips pages' second lines go from as low as 2.5:1 to 4.5:1, every clips and Arrange page reads in the trial light palette); the app's Sequencer, Play, Pads and Looper (the step chips' white and the lock lanes' lightening from `highlight`, their words through `ink_on` and `t.readable`, the pads' type badges and knob labels through `t.readable`, labels on lit buttons through `ink_on` instead of the page background, a muted speaker's red cross made to read on its button (`draw::speaker_on`), corners through `t.round`; the trial light palette's unreadable runs fall to 59, each one also unreadable in Default) |
| 3 | Text sizes, `gap` and `pad` leave themes (owner, 22 Nov: consistent sizes everywhere so no theme can break a layout); the 8 touch areas that assume 14 px padding are fixed | #783: [`sizes.rs`](../../../rust/mpfree-ui/src/sizes.rs) holds the sizes, `gap`, `pad` and the faces; every theme has them (Console's smaller text and spacing are gone, so its pages lay out as Default's do), theme files no longer set them and an older file's lines are ignored with a note; the touch areas that assume 14 px padding are right by construction, since `pad` is 14 in every theme |
| 4 | `Theme::light()` and `themes/light.theme`, Light after Default in Prefs > Appearance, golden screens for Light, the manual page "Making your own theme" (shown to the owner on contact sheets first) | #783: Light is built in after Default (`mpfree --theme light`), its every key in [`themes/light.theme`](../../../rust/mpfree-ui/themes/light.theme); `text_inverse` follows `light_dark({surface}, {bg})` so a light theme's inverse text is its palest colour; `cargo test -p mpfree-app --test cli` draws every screen in Light against [`golden/screens-light.txt`](../../../rust/mpfree-app/golden/screens-light.txt) as well as Default; the contrast check lists Light's 28 pages with faint text under 3:1, each also under 3:1 in Default; the floor (`Theme::unusable`): a theme file no one could read is left out and the default theme shows, logged and counted on the Debug page |
| 5 | Theme writers' tools: problems on the Appearance row, `--check-theme`, a contact sheet script, commented example themes, and a "Keep this theme?" prompt after picking a theme that goes back to the old one after 15 seconds untouched (owner, 22 Nov; a Prefs change, so it goes through the layout spec) | this change: `mpfree --check-theme FILE`, `mpfree --contact-sheet DIR`, two more commented example themes (see [Theme files](#theme-files)), Keep this theme?, and theme problems on the Appearance panel with the themes folder read the forgiving way (the owner's pick on 22 Nov 07:17Z); then two the owner asked for on 22 Nov at 08:19Z and 08:20Z: **Slate**, built in between Default and Light, and Synthwave redone (see [The built-in themes](#the-built-in-themes)) |

Flexible layouts and user-made controls are a separate, later piece of work (owner, 22 Nov), not part of themes.

**MPC plugin skins.** The plugin's own art is drawn as its makers drew it, in every theme. MPFree's own pop-ups over it (the big knob editor and the list of choices) follow the theme (owner's pick, 22 Nov), in the UI font, with guards so a theme can't make them unusable ([`mpfree-mpcskin/src/look.rs`](../../../rust/mpfree-mpcskin/src/look.rs)): every colour pair a pop-up draws is checked when the theme is set (text 4.5:1 on what is under it, the knob's arc and pointer 3:1) and a pair that falls short is moved just far enough towards the theme's text or inverse text (black or white if the theme's own two are too alike); the panel is always solid, and its edge is measured against the dimmed skin round it and drawn to stand out 3:1. The Q-Link bank's frame, MPFree's mark on the art itself, measures the skin pixels under it and takes whichever theme ink reads, see-through, else a two-tone line. Every move is a rescue: logged when the theme is set (`screen: theme NAME: N colour(s) on the skin's pop-ups moved so they read: ...`) and counted on the Debug page with the two-tone frames; the built-in themes need none. Tests draw the pop-ups in every built-in theme and a torture theme (every colour alike, see-through panel) over very dark, very bright and mid-grey stand-in skins. Two faults are fixed with it: the panel is centred in the part of the MPC's screen MPFree shows (its top used to sit under the top bar), and a list names every choice from the skin's replacement strings (it used to show numbers for all but the current one).

## The built-in themes

| Theme | Look | Effects | Startup screen |
| --- | --- | --- | --- |
| Default | The current dark studio look, blue accent. | none | Synthwave's startup screen (Jay, 2026-11-11, from round 8). The app keeps the default look. The spiral galaxy (`mpfree-splash`) stays available. |
| Slate | Blue-grey panels from the owner's slate palette (#313B47, #4B5B6C, #6D8196, #8FA8C1, #C2CDDC), between Default and Light; light words, a teal accent, pastel track colours that read on the grey. A version with dark words on #6D8196 was drawn too: on a mid grey, dim and faint words must be nearly as dark as the main ones to stay readable, and colours go muddy. | none | Synthwave's, like Default. |
| Light | Near-white panels on a pale grey page, dark text, the same blue accent made deep enough to read on white; round corners. | none | Synthwave's, like Default. |
| Console | Phosphor green on near-black, in the normal UI font. | Cut corners, outline and text glow, scanline backdrop, kit colours tinted green. | A chaotic swirl of characters in many shades of green settles, arm by arm, into "MPFree OS" and "v0.1" in the same blade letters as the other screens, drawn as a grid of terminal characters (VT323), one per cell with none overlapping, each picked to match the shape and shading of its cell; then a prompt types out. Scanlines and bloom. |
| Synthwave | Hot pink and cyan on deep violet. Since 22 Nov the outlines are a quiet violet, so pink marks only what is picked; faint words are lighter (runs under 3:1 from 313 to 62); errors and recording are a sunset red, not a pink beside the accent; Ready is a neon mint, not the second highlight's cyan; the Q-Links, graphs and EQ bands take its own neon track colours. | Neon glow on outlines, translucent panels over a grid. | A striped sun rises over rolling, wavy neon hills and a racing grid; "MPFree OS" in MPFree's own blade letters (angular, leaning strokes with torn, jagged starts and knife-point ends), yellow to pink over a hard drop shadow, is cut on over the sun and a pink slash whips under it; then "v0.1" flickers on below. No palm trees, no stripes in the letters. |
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

1. **No hex colours in drawing code** (the [fixed-colour check](#checks) holds the line). Use core colours and roles; for a set of distinct colours use `t.track(i)` or `t.series(i)`, for the Q-Links `t.qlink(i)` (with `t.readable(t.qlink(i), fill)` for a knob's number as text). `mpfree-synth-ui`'s panels build tiles and cards before they know the theme, so they keep a `Paint` (`QLINK_COLORS`, `TEXT`, `TEXT_DIM`, `TEXT_FAINT`, `ON_FILL`, or `Paint::ACCENT` and the rest) that the widgets resolve when painting (see [synths.md](synths.md)). That also covers `mpfree-fx-ui`, which uses the same widgets.
2. **Content colours through `t.swatch(c)`** (kit colours, pad colours, user-chosen track colours): `.fill` for fills (it goes through `t.content(c)`, so tinting themes can pull them in), `.text` for text in the colour, `.ink` for a label on it.
3. **Panels with `cx.panel(r)` / `cx.titled_panel(..)`**, buttons and pads with `fill_round_rect` at `t.radius` (or a little less): the corner style follows. A corner drawn at a fixed size goes through `t.round(n)`; a circle, dot or pill keeps its own radius.
4. **`t.bg` means "background".** A fill in exactly `t.bg` becomes the theme's backdrop. To paint a solid colour that happens to look like the background, use `t.surface` or a token, not `t.bg`.
5. **Text colour on accent fills is `t.on_accent`;** on any other coloured fill it is `t.ink_on(fill)`, never `t.bg`.
6. Previews: add `--theme` to the crate's preview binary and render every theme in tests (cheap, catches unreadable combinations).

## Later

- Startup screens in the boot path: the test stick runs `mpfree-startup --framebuffer` (from 12 Nov, in place of `mpfree-splash`), so it shows the default theme's startup screen, Synthwave's. It starts before the data partition is mounted, so it can't read the theme picked in Prefs yet; `--data /run/mpfree-data` is ready for when it can.
- With the GLES canvas, the same effects become shaders (bloom, scanlines, real blur) with no theme or widget changes.
- Optional per-theme sounds (UI clicks) and pad-light palettes.
