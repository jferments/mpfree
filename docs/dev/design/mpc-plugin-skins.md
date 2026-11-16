<!--
covers: rust/mpfree-mpcskin rust/mpfree-app/src/skin_page.rs rust/mpfree-plughost/src/wire.rs
checked: 2026-11-15 742176c
-->

# MPC plugin skins

**Status:** 2026-11-15. Built and tested on the workstation: every one of the 185 skins in the Open MPC catalog's releases (833 tabs) reads and draws without an error, and the ones checked against their makers' screenshots match. The Synth tab shows an MPC plugin instrument's skin full width (Jay, 15 Nov 22:22Z). Not yet: MPC plugin effects in the chain editor, envelope curves, animated film strips, and a run on the MPC.

An MPC plugin brings the screen its makers drew for it: `Plugin Skins/TUI.json` (the components, their layout and what each control is bound to), `Q-Links.json` (which parameter each Q-Link turns on each tab) and PNG pictures. [`rust/mpfree-mpcskin`](../../../rust/mpfree-mpcskin) reads them and draws them as MPC OS 3.10.1 does, following the inMusic SDK's `docs/ui-reference.md` (MIT) and checked against the catalog's skins. Nothing of Akai's is read or copied: a skin's `importFiles` that point into Akai's own content (189 of the catalog's skins import Akai's "Generic Knob Overlay" and "Generic Menu Overlay") are skipped, and MPFree draws its own knob and menu overlays in their place.

## The crate

| Module | What it does |
|---|---|
| [`json.rs`](../../../rust/mpfree-mpcskin/src/json.rs) | A JSON reader that keeps duplicate keys (overlay files repeat `version`; the first one counts, as on the MPC). |
| [`inflate.rs`](../../../rust/mpfree-mpcskin/src/inflate.rs), [`png.rs`](../../../rust/mpfree-mpcskin/src/png.rs) | Our own inflate and PNG decoder: every colour type and depth, palettes and `tRNS`, interlacing, CRC checks, premultiplied ARGB out. All 34,058 pictures in the catalog's skins decode. No new dependency. |
| [`model.rs`](../../../rust/mpfree-mpcskin/src/model.rs) | The skin: components, their entries (widget, bounds, focus, visibility, handle remapping with `*` wildcards and `{Increment}` repeats), tabs (v1 to v3), partial descriptions repeated over the parameters, replacement strings. |
| [`handle.rs`](../../../rust/mpfree-mpcskin/src/handle.rs) | Handles (`Parameter 3`, `PageIndex 1,1,4`, `Local/2/showAdvanced`, `Fixed/...`) and their modifiers (`Invert`, `AsDB`, `IndexedEnabling`, `Max[...]`, ...), evaluated to a value, its steps and its text. |
| [`qlinks.rs`](../../../rust/mpfree-mpcskin/src/qlinks.rs) | `Q-Links.json` (v4): each tab's 16 knobs and bank direction, Program Mode knobs; parameters in order when a skin has none. Variant files are picked for the unit: `TUI - MPC One.json`, `TUI - MPC.json`, `TUI - 1by4.json`, then `TUI.json` (the MPC One has its four Q-Links in a column). |
| [`paint.rs`](../../../rust/mpfree-mpcskin/src/paint.rs) | Pictures (copied pixel for pixel at their own size, else filtered), film strips, text in the skin's fonts with JUCE's sizing (a font's height is its ascent plus descent). Pictures are decoded when first drawn and kept up to 160 MB, least recently drawn out first; a path outside the plugin's folder is refused. |
| [`view.rs`](../../../rust/mpfree-mpcskin/src/view.rs) | A skin on screen: lays a tab out, draws it, and turns touches, the wheel and the Q-Links into parameter edits (`Edit::Begin`, `Set`, `End`). Mouse Down, Double Click and Enter actions (Q-Link, Show Overlay, Toggle Switch), focus, whenVisible and the invalid rules, buttons in groups, knobs (film strip, rotating picture, generic), sliders, menus, XY pads, the Q-Link column's highlight. |
| [`fonts/`](../../../rust/mpfree-mpcskin/fonts/README.md) | Titillium Web and Roboto subsets (OFL), the fonts skins name. Anything else (Akai's Nexa) is drawn in Titillium Web. |

`cargo run -p mpfree-mpcskin --example render -- <plugin folder> <out dir> [value]` draws every tab of a plugin folder's skin to PNG files with every parameter at one value (no plugin is loaded), and prints how long each took. That is how the catalog was checked: Density2, Acid, Modal Voice, PolyWerk and Clementine XT were compared with the screenshots their makers publish, and match apart from the values. Values matter: at 0.5 every "popup open" parameter is on, so Clementine XT's menus all show open; at 0 they are shut, as on the MPC.

## On the Synth tab

[`mpfree-app/src/skin_page.rs`](../../../rust/mpfree-app/src/skin_page.rs), layout spec section 4g:

- **When.** At the start the app sends the GUI the MPC plugins' folders from the rack (`ToUi::PluginFolders`). When the Synth tab's node is an `mpc:` plugin with a skin, the GUI reads it (`skin_page::load`) and shows the `Synth skin` page: the top bar where every page has it, the skin at (0, top bar's bottom) and 1280 x 628, its tabs (one button per function key, a tap stepping through the key's sub-tabs), the hint line. The page's widgets sit at exact rectangles (`Ui::add_page_slots`, new in `mpfree-ui`), since the layout language puts the theme's gap between slots. A skin that doesn't read, or has no tabs, is said in the log and MPFree's own page shows, as the MPC shows its default pages for a plugin without a skin.
- **Drawing.** `SkinWidget` draws the skin into a picture of its own when something changed (a touch, a value, a tab) and copies it to the screen. An open overlay is drawn on a 1280 x 800 picture as on the MPC, and the tab's part of it shown.
- **Values and texts.** A VST 2 plugin only gives text for its current values, and its values can change by themselves (a program load), so the GUI asks the plugin process for all of them a few times a second while the skin shows (`FromUi::SkinValues`; the builder thread calls `Rack::values`, which sends the plugin process `ToChild::Values` and gets every value and display text back in one answer). After an edit it waits 60 ms (the edit reaching the plugin) and asks again; an answer to an ask from before the last edit is dropped, so a knob doesn't jump back under the finger.
- **Edits** go out as the Synth page's own `SynthAction::Set`, so the audio thread, undo and saving see nothing new. Q-Links 1 to 4 (`ToUi::SynthQLink`) turn the knobs `Q-Links.json` gives the tab's bank, a detent a hundredth of the range or one step of a discrete parameter; a tap on a control picks its bank, as on the MPC. The wheel picks the skin's focusable controls in reading order and, after the push, turns the picked one.
- **MPFree page** (the right-hand button) shows MPFree's own Synth page for the plugin, with a "Plugin skin" button under the instrument card (`synth_with_chip`) to come back.

## Cost

On the workstation, drawing a tab again takes 3 ms (median; 38 ms for the busiest, CSXXL), and the first draw of a tab, which decodes its pictures, 20 ms (median; up to about 1 s for the largest skins). The MPC One's Cortex-A17 cores are perhaps five to eight times slower, so the first look at a big skin's tab can take seconds there; decoding in the background is the next step if it does. A skin's pictures take 28 MB decoded (median; 182 MB for PolyWerk, where the 160 MB budget drops the least recently drawn).

## Not yet

1. **Effects.** MPC plugin effects in the chain editor still show the generic list: the insert has to know its plugin instance to ask for values and texts.
2. **Envelope** widgets draw their background only, not the curve; **animated** film strips show their first frame.
3. **Overlays** a skin brings of its own (rare; most use Akai's generic ones, which MPFree draws itself) are cut to the tab's area.
4. **On the MPC.** Timings and memory on the device, on Jay's go with the soft-boot stick.
