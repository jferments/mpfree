# The browser and projects

**Status:** built on the workstation (crates `mpfree-library` and `mpfree-browser-ui`), not yet in the pad player. The design, its research and the open decisions are in the plan doc: https://claude.ai/artifact/DQbPuih2LPA4pUkQUx3QyW. Controls decision 13 (Jay, 2026-11-11): BROWSE opens the browser, SHIFT + BROWSE saves the project.

## What it does

BROWSE opens the browser over the current page, under the top bar, as Prefs and the mixer do. It is not a tab. BROWSE again closes it. SHIFT + BROWSE saves the project from any page.

- **Places** on the left: Projects, Samples, Kits, Instruments, Patterns, Recordings; Recent and Favourites; Factory (the kits made from the owner's own backup, read only); Trash.
- **The list** in the middle: folders first, then files, in natural order (`Kick 2` before `Kick 10`). Back, the path, New folder and Paste at the top. The filter chips at the bottom (All, Audio, Projects, Kits). A line under them says what just happened, or why something failed.
- **The selected item** on the right: for a sound, its length, rate, bits and channels and a quick waveform; for a project, its tempo and when it was saved. Then its buttons: Load, Load to pad, Audition, Favourite, Rename, Copy, Cut, Delete (Open for folders and projects, Put back in the Trash).
- **Audition** is on by default: moving the selection to a sound plays it once through the preview path. A list arriving never plays anything by itself.
- **Delete** asks first, with the name, and moves the item to the Trash. Empty Trash asks again.
- **Rename and New folder** use an on-screen keyboard. Names may have spaces and capitals. They may not have `/ \ : * ? " < > |`, a leading dot or space, or a trailing dot or space, so they also fit a FAT stick.
- **Saving** needs no typing: the first save names the project for the time (`Project 2026-11-11 21.04`), and Rename or Save as changes it.

| Control while the browser is open | Does |
|---|---|
| Data dial | Moves the selection |
| SHIFT + dial | Moves through the places |
| Dial push | Opens a folder or project, loads anything else, answers yes to a question |
| `-` / `+` | Up a folder / into the selected folder |
| PLAY | Hears the selected sound |
| ERASE | Delete (asks first) |
| Q-Links 1 to 4 | Scroll, filter, preview level, place |

## Pickers

Jay, 2026-11-11 23:16Z: BROWSE stays the full file manager, and each kind of thing also gets a focused picker opened from where it is used. A picker is the same browser narrowed (`Browser::open_picker(Pick)`), in the same place under the top bar. It offers only Load (Open for projects), Audition, Favourite and Cancel; nothing is renamed, moved or deleted there. When it closes, the full browser is back where it was.

| Picker | Opened from | Shows | Does |
|---|---|---|---|
| `Pick::Preset { engine }` | the Synth page | `Instruments/<engine>/`, and that engine's presets in Recent and Favourites | Each turn of the dial sends `TryPreset` so the preset can be played on the pads; Load sends `LoadPreset`; Cancel sends `RevertPreset` |
| `Pick::Sample` | the Sample page or a pad | Samples, Recordings, and sounds in Recent and Favourites | Auditions as you move; Load sends `LoadSample` for the pad it was opened for |
| `Pick::Project` | Open project | Projects | Opening asks first while `set_unsaved(true)`: "Unsaved changes. Open ... anyway?" (the full browser asks too) |

Every picker ends with `PickerClosed { loaded }`. BROWSE while a picker is open should call `cancel_picker`. The hint line is `PICKER_HINT`. Screens 12 to 14 in the previews.

## Where files go (rule zero)

Everything is written to `library/` inside the data folder the app is given with `--data`, which is the test stick's MPFREE-DATA partition (`/run/mpfree-data`). The eMMC and the SD card are never written. The storage spec's "Proposed" section adds `library/` beside the store's `settings/` and `files/` and grows the partition. That needs Jay's go.

```text
library/
  Projects/<name>/        project.txt, parts/<owner>.txt, samples/<hash>.<ext>, versions/<id>/
  Samples/  Kits/  Patterns/  Recordings/
  Instruments/<engine>/<name>.preset   user presets (Jay, 2026-11-11 22:47Z)
  .trash/<id>/<name>      with origin.txt (where it was, when)
```

- A location is a place plus checked names (`Loc`), never a typed path. Every folder on the way must be a real folder; links are never followed for reads or writes.
- Whole or nothing: a new or copied file is written to `NAME.part`, synced and renamed; a copied folder is built under a hidden name and renamed.
- Every file operation runs on the `mpfree-library` worker thread, never on the UI or audio thread.

## Instrument presets

`Instruments/` holds one folder per engine, named as the engine shows it (Poly, FM, Wavetable, Pluck, Drums), with that engine's user presets as `<name>.preset` text files. The synth thread's `mpfree-instruments` turns a node into preset text and back and does no file work of its own: it saves with `Library::write_new` (whole or nothing) and lists through the worker, so the Trash and name checks stay here. Load on a preset sends `LoadPreset { loc, path, engine }`.

## Projects

A project is one folder, complete in itself, readable on any computer.

- `project.txt`: `mpfree-project 1`, then `name`, `tempo`, `created`, `saved`, `app`. Lines it does not know are kept.
- `parts/<owner>.txt`: one text file per engine. An engine implements `mpfree_library::Part` (`owner`, `save`, `load`). The browser never reads a part. A part no engine claims is kept when the project is saved again (`Snapshot::keep_unknown`).
- `samples/`: every sound the project uses, copied in and named by a 128-bit hash of its bytes and its length, so the same sound is stored once and a project never misses a sample because a drive is gone. Two different sounds that hash the same are told apart by comparing bytes.
- `versions/<id>/`: the last 10 saves before this one, each with its `project.txt` and `parts/`.
- A save writes the new parts to `.parts-new/`, syncs them, moves the old `parts/` into `versions/`, renames the new parts into place, then replaces `project.txt` through a `.part` file. Opening a project finishes or undoes a save that was cut off.

## For the app (Hello MPC thread wires it)

```rust
let lib = mpfree_library::Library::at(Path::new(data_dir))?;   // the --data folder
let mut browser = mpfree_browser_ui::Browser::new(lib);         // starts the worker
browser.set_factory(kit_names);                                  // from kits.bin
browser.load_lists(&store_text("browser", "lists.txt"));        // Recent, Favourites
mpfree_browser_ui::add(&mut ui, &browser)?;                      // the "browser" page
// each frame while open:
for a in mpfree_browser_ui::step(&mut ui, &mut browser) { /* BrowseAction */ }
// SHIFT + BROWSE anywhere:
browser.save(Snapshot::from_parts(meta, &[&transport, ...]), now_utc);
```

The sampler's part comes with the sounds it names, which the save writes into the project's `samples/` first where they aren't there yet (`Library::save_project_with`; a sound that can't be written leaves the last save standing):

```rust
let (text, sounds) = sample_page.save_part();                   // SamplePage in mpfree-hello
snap.parts.push(("sampler".into(), text));
let samples = sounds
    .into_iter()
    .map(|(file, s)| NewSample::new(file, Arc::new(move |w: &mut dyn Write| mpfree_sampler::wav::write(&s, w))))
    .collect();
browser.save_with(snap, samples, now_utc);
// BrowseAction::ProjectOpened { loc, snap }:
if let Some(text) = snap.part("sampler") {
    let lib = Library::at(data_dir)?;
    sample_page.open_part(text.to_string(), Box::new(move |f| {
        lib.sample_path(&loc, f).and_then(std::fs::read).map_err(|e| e.to_string())
    }));
}
// BrowseAction::LoadSample { path, .. } => sample_page.load_file(path, false),
// BrowseAction::LoadToPad { path, .. } => sample_page.load_file(path, true),
```

`BrowseAction`s for the app: `LoadSample` / `LoadToPad` / `LoadKit` / `LoadFactoryKit` (to the sampler thread's loader), `LoadPreset` (with the engine's folder name, to the synth thread's `mpfree-instruments`), `TryPreset` / `RevertPreset` / `PickerClosed` (pickers), `Audition` / `StopAudition` (the preview path), `ProjectOpened` (give each engine its part), `ProjectSaved`, `SaveRequested` (Save as got a name: call `save`), `SaveLists` (write it to the store as `files/browser/lists.txt`).

When `mpfree-library` is linked into the pad player, `tools/mpc-one/tests/test_hello_mpc.py` needs `rust/mpfree-library/src` in `LINKED_SRC`, and its write sites (all in `lib.rs` and `project.rs`, all under the library folder) added to the reviewed list.

## Not yet

- The pad player wiring (Hello MPC thread). The sampler's side is ready: its part, its sounds, Load and Load to pad (above).
- `LoadKit` and `LoadFactoryKit`: a kit file's format and saving a kit on its own come later (sampler thread).
- Each engine's part (`transport`, `clips`, `sampler`, `synth`, `mixer`, `fx`, `mod`, `arp`, `looper` ...): each thread adds its own.
- A Versions list on the page, autosave, search, sorting by date.
- AIFF and FLAC waveforms in the browser (their headers are read; WAV only gets the quick waveform).
- Sharing the library over Ethernet (WebDAV and a web page), a desktop `mpfree-sync`, and USB file mode: plan section 9, decisions 9 to 11.
