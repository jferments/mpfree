# The browser and projects

**Status:** built (crates `mpfree-library` and `mpfree-browser-ui`). In the pad player since 12 Nov 2026 as the Pads tab's Browse view (`Pick::Pads`, layout spec section 4e); the full browser over every page (BROWSE) is not wired yet. The design, its research and the open decisions are in the plan doc: https://claude.ai/artifact/DQbPuih2LPA4pUkQUx3QyW. Controls decision 13 (Jay, 2026-11-11): BROWSE opens the browser, SHIFT + BROWSE saves the project.

## What it does

BROWSE opens the browser over the current page, under the top bar, as Prefs and the mixer do. It is not a tab. BROWSE again closes it. SHIFT + BROWSE saves the project from any page.

- **Places** on the left: Projects, Samples, Kits, Instruments, Patterns, Recordings; Recent and Favourites; Factory (the kits made from the owner's own backup, read only); Trash.
- **The list** in the middle: folders first, then files, in natural order (`Kick 2` before `Kick 10`). Back, the path, New folder and Paste at the top. The filter chips at the bottom (All, Audio, Projects, Kits), then **Search** and the order chip (**A to Z** or **Newest**: folders first either way, then the last changed first). A line under them says what just happened, or why something failed.
- **Search** opens the keyboard. The text is looked for, in any case, in every name in the library except the Trash, and not inside projects. Results show where each item is, with the same buttons as Recent (no Rename or Cut), and the filters still apply. Back returns to the folder. At most 200 results; the search stops after 4000 folders so it costs a bounded time on the worker.
- **The selected item** on the right: for a sound, its length, rate, bits and channels and a quick waveform (WAV, AIFF and uncompressed AIFF-C read at each column's place; FLAC decodes the one frame at each column's place, found by guessing from the bytes, so a 3-minute FLAC reads about a third of the file: `mpfree_flac::overview`); for a project, its tempo and when it was saved. Then its buttons: Load, Load to pad, Audition, Favourite, Rename, Copy, Cut, Delete (Open for folders and projects, plus Earlier saves for a project; Put back in the Trash).
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
- `versions/<id>/`: the last 10 saves before this one, each with its `project.txt` and `parts/`. **Earlier saves** on a project lists them, newest first, by when they were saved. Open on one gives its parts to the app in place of the project's (it asks first over unsaved work) and marks the work unsaved; the next save makes it the newest, and the save it replaces becomes an earlier save, so nothing is lost. An earlier save's row only opens: Delete, Rename and the rest do nothing there.
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
let (text, sounds) = sample_page.save_part();                   // SamplePage in mpfree-app
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

`mpfree-library` and `mpfree-browser-ui` are linked into the pad player, so `tools/mpc-one/tests/test_app_mpc.py` reads them (`LINKED_SRC`) and lists the library's write sites, all in `lib.rs` and `project.rs` and all under the library folder, line by line (`LIBRARY_NEEDLES`, `LIBRARY_OPENS`). A change to one of them changes that list in the same pull request.

**In the pad player (the Pads tab).** With `--data`, the app sends the screen thread the folder and the saved lists (`ToUi::Library`); the screen opens `Library::at` (which makes `library/` and its place folders when missing) and a `Browser` on the library's worker thread, opened with `Pick::Pads`. `mpfree_browser_ui::step` runs each tick while Browse shows and after each touch. Its actions: `LoadSample` and `LoadToPad` go to `SamplePage::load_file`; `Audition` reads the file on a thread (`sample_page::read_in_thread`) and hands the sound to the audio thread in a box (`FromUi::Preview`, `mpfree-app/src/preview.rs`), which mixes it into the main out at the preview level and gives the box back to be freed on the screen thread (`ToUi::PreviewBack`); `StopAudition` fades it out over 256 frames (`FromUi::PreviewStop`); `SaveLists` goes to the store (`files/browser/lists.txt`). The others are logged as not wired on the Pads tab. Without `--data`, the Browse view shows a card that says so.

## Not yet

- The full browser over every page (BROWSE, SHIFT + BROWSE to save) and the other pickers in the pad player (app thread). The Pads tab's picker is wired (above).
- `LoadKit` and `LoadFactoryKit`: a kit file's format and saving a kit on its own come later (sampler thread).
- Each engine's part (`transport`, `clips`, `sampler`, `synth`, `mixer`, `fx`, `mod`, `arp`, `looper` ...): each thread adds its own.
- Autosave.
- Sharing the library over Ethernet (WebDAV and a web page), a desktop `mpfree-sync`, and USB file mode: plan section 9, decisions 9 to 11.
