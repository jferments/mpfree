# Sound presets (`rust/mpfree-instruments`)

A sound on any instrument can be saved as a preset file and loaded back, on any engine: Poly, FM, Wavetable, Pluck, Keys, Drawbar, Squelch, Analog, Strings, Bass and Drums today, and any other node that lists its parameters. This is piece 3 of the plan "Four synth features" (2026-11-11). Jay chose to keep user presets in the library: `library/Instruments/<engine>/<name>.preset`, where `<engine>` is the engine's name (Poly, FM, Wavetable, Pluck, Drums).

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
- **On the pads** (`mpfree-app`, `synth.rs`), the app's synth holds each engine as a sound (`Sound<Poly>` and so on), so a preset that brings effects and macros plays them, and it reads and plays user presets for the browser's preset picker (`Pick::Preset`). The picker names a file's engine by its folder under Instruments, the engine's name (`Poly`, `FM`, `Wavetable`, `Pluck`, `Drums`); `Engine::from_folder` maps it back, and a file straight in Instruments goes by its `engine` line.
  - `try_preset(folder, text)` (the picker's `TryPreset`, one per dial turn) plays the file on the engine on the pads, effects and macros too, and keeps the sound from before the first try (as text, with its design).
  - `revert_preset()` (`RevertPreset`, on Cancel or BROWSE) puts that sound back, every setting to the bit.
  - `load_preset_file(folder, text)` (`LoadPreset`) plays the file from then on and forgets the kept sound.
  - `save_preset(name, tags, extra)` gives the folder and the text to write, with `extra` lines such as the Q-Link sets.
  - A Drums kit moves its pads, and they move back on Cancel. A file for another engine, in another engine's folder, or not a preset changes nothing.
  - The app wires these to the browser's actions with the browser page.
- The crate does no file I/O. The library (`mpfree-library`, the browser thread) writes files whole or not at all, refuses a taken name, lists the Instruments place and moves deleted presets to the Trash. Saving makes `Instruments/<engine>` if it is missing, then writes `<name>.preset` with a name the library picked as unique.

## Sounds: an engine with its own effects and macros

A preset can carry more than the engine's settings: up to three effects of its own and up to six macros ([Architecture, section 5](synth-architecture.md#5-what-a-preset-carries)). `Sound` (`mpfree-instruments`, `sound.rs`) plays one: a node that holds the engine's node, the effects in a chain after it (`mpfree_fx::Chain`) and the macros. A plain preset (no effects, no macros) is a sound with nothing added and plays bit for bit as the engine alone.

**In the file**, the design is a few lines beside the settings, and the effects' and macros' settings are ordinary settings:

```text
mpfree-preset = 1
engine = mpfree.keys
name = Space Tines
fx.1 = mpfree.fx.ensemble
macro.1 = Brightness
macro.1.map = tone.tone -40 40 linear
macro.1.map = tone.hardness 30 80 exp
...
fx-1.mix = 42
macros.brightness = 35
```

- **`fx.N`** names slot N's effect by its id in the effects' registry (`mpfree_fx::registry`). Slots run in order, 1 to 3.
- **`macro.N`** names macro N (at most 16 characters). Each **`macro.N.map`** line is one setting it moves: the setting's key, the value at 0 %, the value at 100 %, and the curve (`linear`, `exp`, `log` or `s`; linear when left out). Up to eight a macro. A macro moves its settings in knob positions, so a cutoff sweeps evenly in pitch, and it lands on the two values exactly at its ends.
- **The effects' settings** are keyed by their slot (`fx-1.mix`). **The macros' positions** are `macros.<name>`, 0 to 100 %. Loading sets the macros' positions without moving their settings, so the settings load as they were saved.
- A file with design lines loads in a build without sounds as a plain preset: the lines ride along as unknown lines and are written back.
- What can't be built is refused with a reason: a fourth effect or a seventh macro, an effect the registry doesn't have, a map to a setting the sound doesn't have (or to a macro or a meter), a macro name that is empty or too long.

**As a node**, a sound is the engine to whatever hosts it, with more settings:

- `info()` is the engine's, so the Synth tab draws the engine's own pages, and every id the engine uses keeps its meaning.
- Slot N's effect settings are `61440 + (N − 1) × 1024 + id`, in the group "FX N"; macro K is `64512 + K − 1`, in the group "Macros", 0 to 100 %. A step lock, an LFO or MIDI Learn on any of them needs no new code.
- The macros come first in the sound's list of settings, then the engine's, then each effect's. A host that copies a sound by setting every value in the list's order (a project, a track's copy) so sets each macro before what it moves, and a setting turned after its macro ends as saved.
- Moving a macro moves its settings; a setting turned by hand afterwards stays until the macro moves again (the last move wins). A macro moved on a frame (a lock, automation) moves its settings on that frame.
- The Synth tab puts a node's "Macros" group on its first page, and Q-Links 1 to 4 turn the first four macros (`mpfree_synth_ui::pages::plan`). The effects get a page each after the engine's own.
- Notes, the tuning, load shedding (`cull`), voice counts and the latency go to the engine; the tail is the engine's plus each effect's.
- `Sound<E>` keeps the engine's type: a host that needs the engine's own calls (a factory preset, a drum kit's pads) holds a `Sound<Keys>` and reaches them through `Deref`. `set_design` and `load_text` change a sound in place (the engine keeps its notes; a preset with the same design keeps its effects running); on an error nothing changes. The `Designed` trait gives the same calls on a sound of any engine.
- Building a sound allocates (the chain, the parameter list, made once per design and kept, since nodes hand out `&'static` lists); after `prepare`, `process` never does (`tests/sound_no_alloc.rs`). On the workstation the wrapper adds about 3,000 instructions a block to the engine and its effects (bench case "Sound Keys Init with Ensemble, 8 voices": 254k, against Keys Init's 113k and the Ensemble's 138k).

## Factory presets as files

The engines' factory presets are preset files built into the program, one file an engine in `rust/mpfree-synth/presets/` (`keys.presets`, `bass.presets` and so on; FM's presets are still built in code). A file is the presets' blocks one after the other, each a preset in the format above, so a factory preset is written, reviewed and checked like any other file:

```text
mpfree-preset = 1
engine = mpfree.keys
name = Soft Tines

# Tone
tone.hardness = 30
tone.bell = 25
```

- **A block lists what differs from the engine's defaults** (what the engine's `load_preset` starts from), so the file reads as the sound's recipe and a review sees each change. Init is the first block, with nothing in it.
- `mpfree_synth::factory::read` reads a file once, the first time the engine's `presets()` is called (`presets!` keeps them for the program's life, as the engines hand out `&'static` presets). Each `Preset` keeps its block's text (`Preset::text`), with the lines the engine doesn't read: a sound's effects and macros.
- **A factory sound** is a block with design lines (`fx.N`, `macro.N`, [above](#sounds-an-engine-with-its-own-effects-and-macros)). The app's `Synth::load_preset` plays it as it plays a user's preset (`load_text`), so the effects and macros come with it; a plain block loads on the bare engine as before, and takes away what an earlier sound brought. A track's copy takes the same: `Synth::preset_values` gives the design lines with the values, and `fresh_node` builds the sound.
- The first factory sounds (18 Nov): Phase Tines and Wah Clav on Keys, Hall Strings and Space Strings on Strings, Amp Bass and Tube 808 on Bass. Each has four or five macros, the ones the architecture gives its type ([section 6](synth-architecture.md#on-the-mpc)): Tone, Bark, Tremolo and Effects on Keys (Tone, Wah, Mute and Effects on the clav); Crescendo, Sustain, Ensemble, Brightness and Space on Strings; Cutoff, Env amount, Drive and Glide on Amp Bass; Drop, Decay, Drive and Tone on Tube 808. Each macro's saved place agrees with the settings it moves, so its first turn carries on from the sound as saved.
- A Bass preset never sets the player's Level, so the two basses are levelled with their effect's own output.

## Tests

- Every factory sound of every engine saved as text and loaded into a fresh engine has every setting identical to the bit and plays a phrase identically to the bit.
- Factory files (`mpfree-synth`, `factory.rs`): every block of every engine's file reads, Init first, no two presets with one name, every value one the engine has; a block that doesn't read is left out and said. Moving the presets from Rust tables to files changed nothing: every setting of all 155 presets loaded to the same bits before and after.
- Factory sounds (`mpfree-instruments`, `tests/factory_sounds.rs`): every factory preset loads as a sound, giving the engine the very values its own `load_preset` gives; no line in a block goes unread (a misspelt setting would otherwise ride along as an unknown line and do nothing); a block is plain exactly when it has no design lines; and every macro set to its saved place leaves every setting where the file has it.
- Every engine with every setting at an awkward value does the same; this is the test that fails if a number loses a digit.
- Native engines name every setting (no `id.` keys) and save every writable one.
- A setting missing from the file takes its default; a preset for another engine is refused and changes nothing.
- Keys, the id fallback, choices and switches by name or number, unknown lines kept and written back, names and tags kept on one line, and every refusal with its reason.
- Sounds (`sound.rs`): a sound with nothing added plays bit for bit as its engine; its effects play bit for bit as the same chain by hand; effect settings get their own ids and groups, and one design gets one parameter list; a macro moves its settings along their curves, on the frame it moves; a sound saved as text loads back to the bit (settings, effects, macros and their positions); a plain preset loads as a sound with nothing added; every refusal says why. A sound's `process` allocates nothing through a busy session with every setting automated (`tests/sound_no_alloc.rs`).
- In `mpfree-app`: a user preset with its own effects and macros plays them on the pads, Cancel brings the plain sound back to the bit, a plain factory preset takes them away, and a project keeps them (`synth.rs`); a factory sound brings its effects and macros on the pads and to a track's copy, which plays it to the bit (`synth.rs`); a track's copy of such a sound plays it as the pads do, and a design that can't be built makes nothing (`synth.rs`); a track's sound reads back from the project to the bit (`project.rs`).
- In `mpfree-app`'s `synth.rs`: trying two presets and cancelling brings back every setting to the bit; Load after trying keeps the tried sound; another engine's file or folder, and a file that isn't a preset, change nothing; a file straight in Instruments goes by its engine line; Q-Link sets ride along in a save; a Drums kit tried moves the pads and Cancel moves them back.

## Not yet

- **More factory sounds.** Six factory presets bring effects and macros so far; the rest of each engine's presets get theirs with each wave's preset work. Send hints, the category and the play lines (section 5's table) come with the preset browser (roadmap row 9, [Architecture](synth-architecture.md#7-roadmap)). FM's presets are still built in code.

- A Wavetable sound on a user table (one made from a sample) saves the table choice "User" but not the table itself; the preset will name the table's file once the library holds it.
- Plugins: a CLAP plugin's own state chunk is not saved in a sound preset yet. A plugin's own presets (its factory and user files, listed and loaded through CLAP) are the plugin's, not this format: see [Plugins](plugins.md#in-the-app).
- The catalogue (factory presets with tags, search, favourites) and the Save button come next. The picker's try, cancel and load are built on the app's side (`Synth::try_preset`, `revert_preset`, `load_preset_file`, and `save_preset` for the text a Save writes). The app links the library since the Pads tab (PR #250), but its browser offers sounds only and says "a preset is not wired" for the preset actions, so the Synth tab still needs to open the picker and hand those actions to the synth.
