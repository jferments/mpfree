# Sound presets (`rust/mpfree-instruments`)

A sound on any instrument can be saved as a preset file and loaded back, on any engine: Poly, FM, Wavetable, Pluck, Keys, Drawbar, Squelch, Analog and Drums today, and any other node that lists its parameters. This is piece 3 of the plan "Four synth features" (2026-11-11). Jay chose to keep user presets in the library: `library/Instruments/<engine>/<name>.preset`, where `<engine>` is the engine's name (Poly, FM, Wavetable, Pluck, Drums).

## The file

Plain `key = value` text, so a person can read and edit it:

```text
mpfree-preset = 1
engine = mpfree.poly
name = Dust Pad
tags = pad, warm

# Osc 1
osc-1.wave = Saw
osc-1.octave = 0
...
# Filter
filter.cutoff = 1840
...
qset = 31 32 34 33 Tone
```

- **The first line** names the format and its version. A file from a newer format is refused with a reason ("saved by a newer MPFree"), and so is a file for another engine.
- **`engine`** is the node's id (`mpfree.poly`), so a preset still loads after it is moved to another folder. `engine_of` reads just this line, so the app can pick the engine before reading the rest.
- **`name`** is one line of at most 40 characters. **`tags`** are up to 8, comma separated, each at most 20 characters; the instrument browser filters on them.
- **Settings by name.** A setting's key is its group and name in lower case with `-` between words (`filter.cutoff`, `filter-env.attack`, `op-3.level`), so a preset reads like the Synth tab. A setting with no group is `param.<name>`. Where two settings of one node would share a key (a plugin's, say), both use their ids (`id.31`). A comment line names each group.
- **Values.** Choices are written by name (`Saw`) and switches as `on` or `off`; a number works for either. Numbers are the shortest text that reads back to the same `f32`, so a sound loads to the bit. Meters are not saved.
- **Old and new presets load.** A setting the file doesn't name, such as one added to the engine after the file was saved, takes its default. A value out of range is clamped. A line this build doesn't read is kept as it is and written back on the next save; that is how other owners' lines ride along, such as the Q-Link sets (`qset`, see the Synth tab in [Synths](synths.md)). Blank lines and lines starting with `#` are skipped.
- Renaming a setting would orphan its old key, so a rename needs an alias here when it happens.

## The code

- `Preset::capture(node, name, tags)` takes the node's sound now.
- `Preset::to_text(params)` writes it; `Preset::parse(text, info, params)` reads it, checking the format and the engine.
- `Preset::apply(node)` sets every setting: the preset's value, or the default when it has none, clamped. Sounding notes carry on, as with a factory preset. A preset for another engine changes nothing.
- **On the pads** (`mpfree-app`, `synth.rs`), the app's synth reads and plays user presets for the browser's preset picker (`Pick::Preset`). The picker names a file's engine by its folder under Instruments, the engine's name (`Poly`, `FM`, `Wavetable`, `Pluck`, `Drums`); `Engine::from_folder` maps it back, and a file straight in Instruments goes by its `engine` line.
  - `try_preset(folder, text)` (the picker's `TryPreset`, one per dial turn) plays the file on the engine on the pads and keeps the sound from before the first try.
  - `revert_preset()` (`RevertPreset`, on Cancel or BROWSE) puts that sound back, every setting to the bit.
  - `load_preset_file(folder, text)` (`LoadPreset`) plays the file from then on and forgets the kept sound.
  - `save_preset(name, tags, extra)` gives the folder and the text to write, with `extra` lines such as the Q-Link sets.
  - A Drums kit moves its pads, and they move back on Cancel. A file for another engine, in another engine's folder, or not a preset changes nothing.
  - The app wires these to the browser's actions with the browser page.
- The crate does no file I/O. The library (`mpfree-library`, the browser thread) writes files whole or not at all, refuses a taken name, lists the Instruments place and moves deleted presets to the Trash. Saving makes `Instruments/<engine>` if it is missing, then writes `<name>.preset` with a name the library picked as unique.

## Tests

- Every factory sound of every engine (44) saved as text and loaded into a fresh engine has every setting identical to the bit and plays a phrase identically to the bit.
- Every engine with every setting at an awkward value does the same; this is the test that fails if a number loses a digit.
- Native engines name every setting (no `id.` keys) and save every writable one.
- A setting missing from the file takes its default; a preset for another engine is refused and changes nothing.
- Keys, the id fallback, choices and switches by name or number, unknown lines kept and written back, names and tags kept on one line, and every refusal with its reason.
- In `mpfree-app`'s `synth.rs`: trying two presets and cancelling brings back every setting to the bit; Load after trying keeps the tried sound; another engine's file or folder, and a file that isn't a preset, change nothing; a file straight in Instruments goes by its engine line; Q-Link sets ride along in a save; a Drums kit tried moves the pads and Cancel moves them back.

## Not yet

- A Wavetable sound on a user table (one made from a sample) saves the table choice "User" but not the table itself; the preset will name the table's file once the library holds it.
- Plugins: a CLAP plugin's own state chunk is not saved yet.
- The catalogue (factory presets with tags, search, favourites) and the Save button come next. The picker's try, cancel and load are built on the app's side (`Synth::try_preset`, `revert_preset`, `load_preset_file`, and `save_preset` for the text a Save writes). The app links the library since the Pads tab (PR #250), but its browser offers sounds only and says "a preset is not wired" for the preset actions, so the Synth tab still needs to open the picker and hand those actions to the synth.
