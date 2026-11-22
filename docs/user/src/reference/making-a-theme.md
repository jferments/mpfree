<!--
status: changing
covers: rust/mpfree-ui/src/theme.rs rust/mpfree-ui/src/roles.rs rust/mpfree-ui/themes rust/mpfree-app/src/main.rs rust/mpfree-app/src/app.rs
checked: 2026-11-22 512dc09a
-->

# Making your own theme

A theme sets MPFree's colours and the style of its panels. MPFree has seven built in: **Default**, **Slate**, **Light**, **Console**, **Synthwave**, **Spaceship** and **AbleTown** (**Prefs > Appearance**). You can make your own with a short text file.

A theme changes colours, corners, glow, the pattern behind the panels and the startup screen. It never moves anything. The layout, the text sizes and the font are the same in every theme, so every button and every place you touch stays where it is. The pads' lights keep their colours, and so does a plugin's own skin.

## Your first theme

1. On your computer, make a plain text file named `sunrise.theme`. Use lower-case letters, digits, `.`, `_` or `-` in the name, and end it with `.theme`.
2. Put these three lines in it:

   ```text
   base   = synthwave
   name   = Sunrise
   accent = #FF6C11
   ```

3. Copy it into the folder `files/themes/` on the MPFREE-DATA part of your MPFree stick.
4. Start MPFree. Press **SHIFT + MENU** to open Prefs, pick **Appearance** and press **+** until it says **Sunrise**. The screen changes at once, and the right panel asks **Keep Sunrise?** Press **Keep**, or push the data wheel. If you do nothing for 15 seconds, MPFree goes back to the theme you had, so a theme you can't read never sticks.

That's Synthwave with an orange accent. MPFree remembers your pick at the next start. Theme files come after the built-in themes on the Appearance row, in name order, up to 32 of them.

{{shot: prefs-appearance | Prefs, with Appearance picked.}}

## How a theme file reads

- One setting per line, written `key = value`. A `#` at the start of a line, or after a space, starts a comment.
- **`base`** comes first: the built-in theme to start from (`default`, `slate`, `light`, `console`, `synthwave`, `spaceship` or `abletown`). Anything you don't set stays as that theme has it, so a theme can be one line long.
- **`name`** is what the Appearance row shows. Without one, or with the name of a built-in theme, the row shows the file name.

The quickest way to start is to copy a built-in theme. Each one is written out in full, every setting with its value, in MPFree's source code on [GitHub](https://github.com/jferments/mpfree): search the repository for `light.theme`, say. Copy one, change what you like, and keep only the lines you changed under a `base = ...` line.

## Colours

A colour is `#RRGGBB`, as on the web, or `#RRGGBBAA` to make it see-through (`#15192280` is half see-through). These colours make up a theme:

| Key | What it colours |
|---|---|
| `bg` | The page behind everything. |
| `surface` | Panels and cards. Make it see-through to let the pattern behind it show. |
| `raised` | Buttons and boxes on a panel. |
| `outline` | The lines round panels and buttons. |
| `text` | Words. |
| `text_dim` | Labels, and words that matter less. Button labels use it when the button does nothing right now. |
| `text_faint` | Hints, and the faintest words. |
| `accent` | The main highlight: what is picked, a slider's fill. |
| `accent2` | A second highlight, used sparingly. |
| `on_accent` | Words on the accent. |
| `good`, `warn`, `bad` | Ready or fine; take care; an error. |
| `track` | Eight colours for things that come in a set: the four Q-Links (the first four colours), macros, and graph lines and EQ bands. Write all eight on one line, `track = #2A63D4 #17784C ...`; a shorter list repeats. `track.3 = #A96700` sets one. |

A few more colours have their own keys, and each follows the colours above unless you set it. You rarely need them:

| Key | What it colours | What it follows |
|---|---|---|
| `record` | Recording: the record button, armed inputs, the level while recording. | `bad` |
| `qlink.1` to `qlink.4` | The four Q-Link knobs and their labels. | `track.1` to `track.4` |
| `text_inverse` | Words on a fill too bright (or too dark) for `text`. | `surface` on a light theme, `bg` on a dark one |
| `highlight` | What a pad or button flashes towards when hit, the ring round a picked cell. | `text` |
| `shadow` | Drop shadows, drawn see-through. | `text` on a light theme, `bg` on a dark one |
| `series.1` to `series.8` | Graph lines and EQ bands, in order. | `track.1` to `track.8` |
| `handle` | A switch's knob, a slider's handle. | `surface` on a light theme, `text` on a dark one |
| `cut` | The EQ's low and high cuts. | `text_dim` |
| `grain` | The Sample page's grain window. | `accent2` |

### Colours that follow other colours

Instead of a number, a colour can follow another colour of the theme, so changing one changes the rest with it:

- `{accent}` is the theme's accent (any key above works, such as `{track.2}` or `{text}`).
- `mix({accent}, #FFFFFF, 30%)` is 30% of the way from the accent to white.
- `alpha({accent}, 40%)` is the accent at 40%, see-through.
- `light_dark(#FFFFFF, #000000)` is white on a light theme and black on a dark one. **`mode`** says which yours is: `auto` (the default: a theme whose `bg` is brighter than its `text` is light), `light` or `dark`.

```text
# Sunrise: Synthwave's grid and glow with an orange accent, cut corners
# and dots. The outline, the dots and the first Q-Link follow the accent.
base          = synthwave
name          = Sunrise
accent        = #FF6C11
outline       = alpha({accent}, 66.7%)
backdrop      = dots
backdrop_line = alpha({accent}, 40%)
corners       = chamfer
qlink.1       = {accent}
```

A colour can follow one set further down the file. Two colours that follow each other (`accent = {accent2}` and `accent2 = {accent}`) are a mistake.

## Style

| Key | What it does | Values |
|---|---|---|
| `corners` | The corners of panels and buttons. | `round`, `chamfer` (cut off at an angle) or `square` |
| `radius` | How big the corners are. Every corner on every page follows it; dots and round buttons stay round. | 0 to 40 (Default: 14) |
| `glow` | A soft halo round outlines. | 0 (none) to 255 |
| `text_glow` | A soft halo round words. | 0 to 255 |
| `backdrop` | The pattern behind the panels. | `solid`, `gradient`, `grid`, `dots` or `scanlines` |
| `backdrop_to` | The gradient's second colour. | a colour |
| `backdrop_line` | The colour of the grid, dots or scanlines. | a colour |
| `backdrop_step` | How far apart the lines or dots are, in pixels. | 2 to 400 |
| `tint` | How far kit and pad colours on the screen lean towards the accent (the pads' lights don't change). | 0 to 255 |
| `startup` | The startup screen. | `galaxy`, `console`, `synthwave` or `spaceship` |

Text sizes, spacing and the font aren't theme settings. A theme file written before they were taken out still works: MPFree ignores those lines.

## When something in a theme is wrong

MPFree reads every theme file when it starts. When a line has a mistake, such as a misspelt colour, the theme still shows up on the Appearance row: that one setting stays as the `base` theme has it, and everything else in the file is used.

Pick your theme on **Prefs > Appearance** and the right panel lists what MPFree couldn't use, with the line number:

```text
Sunrise: 1 problem
line 3: accent: expected a colour (#RRGGBB, {name}, mix(), alpha() or light_dark()), got "orange" (kept synthwave's #FF2E97)
```

It also lists lines MPFree ignores (a key it doesn't know, or an old setting such as a text size) and colours that are hard to read against each other. The same lines go in the log: turn on **Prefs > Show developer tools**, open the **Console** on the Dev tab, and look for lines that start `themes:`.

Fix the line, copy the file to the stick again, and start MPFree again.

### A theme no one could read

MPFree won't use a theme whose words can't be read, because then you couldn't read Prefs to pick another theme either. If `text` or `text_dim` is too close to `bg`, to `surface` or to `raised` (under 3:1 contrast, the point where words stop being readable), the file is left out of the Appearance row. The Appearance panel names it and says why, and so does the log:

```text
themes: fog.theme left out: nothing could be read: text on panels (surface) 1.0:1, under the 3:1 every theme must reach
```

If that was the theme you had picked, MPFree starts in the Default theme instead and the log says so. The Dev tab's **CPU by engine** page counts all of this: the theme lines that used the base's value, the theme files left out, and the starts that showed Default in place of the theme you picked.

Good contrast is higher than that: 4.5:1 for words and 3:1 for lines and dim labels is a good aim. The built-in themes all reach it. A website contrast checker will tell you the contrast between two colours.
