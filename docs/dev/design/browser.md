# The browser and projects

**Status:** built (crates `mpfree-library` and `mpfree-browser-ui`). In the pad player since 12 Nov 2026 as the Pads tab's Browse view (`Pick::Pads`, layout spec section 4e), and since 13 Nov as the full browser over every page (BROWSE), with whole projects: SHIFT + BROWSE, Save, Save as, Open and New project (section "Projects in the pad player"). The design, its research and the open decisions are in the plan doc: https://claude.ai/artifact/DQbPuih2LPA4pUkQUx3QyW. Controls decision 13 (Jay, 2026-11-11): BROWSE opens the browser, SHIFT + BROWSE saves the project.

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
  .work/<owner>/          an engine's files for work not saved yet (Library::work_dir)
```

- A location is a place plus checked names (`Loc`), never a typed path. Every folder on the way must be a real folder; links are never followed for reads or writes.
- Whole or nothing: a new or copied file is written to `NAME.part`, synced and renamed; a copied folder is built under a hidden name and renamed. `Library::write_new_unsynced` is the same without the sync, for thousands of files synced once at the end (the creator stick's first start copies Akai's samples that way: two flushes of the stick per file had it crawling).
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
- The Projects place's list has **New project**, **Save as** and **Save** at the top (pickers never show them). Save asks the app for its parts (`BrowseAction::SaveRequested`); Save as asks for a name first; New project asks first over unsaved work (`Confirm::NewProject`), forgets the project and sends `BrowseAction::NewProject`. The app says when the work changes (`Browser::set_unsaved`), so Open and New ask before throwing it away. **Export** sits after Save: it asks the app for the song's facts (`BrowseAction::ExportAsked`) and opens the Export panel over the browser (`Browser::open_export`, `mpfree_browser_ui::export`); the export itself is [export.md](export.md#the-apps-part).
- **Work folders.** `Library::work_dir(owner)` is `library/.work/<owner>/` (made when missing, never shown: it is not a place): where an engine keeps the files of work not saved yet, such as an audio track's takes, on the library's own drive. A save copies them in with `NewSample::file(name, path)` (read without following a link, written as any other sample), so the project keeps its own copy. `Library::clear_work(owner)` removes the files in it (not links or folders), for New project or an open over unsaved work.

### The parts

The pad player's parts, one file each in `parts/`. Each is text: a first line naming its format and version, then lines a reader skips when it doesn't know them, so an older build opens a newer project as far as it can. Code: `mpfree-app/src/project.rs`, `mpfree-seq/src/save.rs`.

| Owner | Engine | Format | What it holds |
|---|---|---|---|
| `clips` | the time engine (`mpfree-seq`) | `mpfree-clips 1` | Every track (name, destination, swing, mute, through-arp, record touch) and its clips: length, speed, swing, quantise, follow action, notes with chance, condition, ratchet and touch, locks, and move lanes. Then the scenes (names, mutes, follow, stored values) and the song (placements, markers, morphs, song lanes). `Project::check` runs on what was read. |
| `mixer` | the mixer (`mpfree-fx`) | `mpfree-mixer 1` | `padmutes`, each strip's controls (`strip t0 1=-6 2=0.25 ...`), and each insert in order: `insert <strip> <uid> <on> <node id> [key=<strip>] [extras]`, its `param` lines and its `state` (base64, from `Node::save_state`). Left out: the master's volume (the Level stays the room's, never a file's), the line input's source, monitor and arm (Prefs), the pads' mute controls (in `padmutes`), and a track's own instrument channel with nothing set on it (`project::untouched_channel`: no effects, every control at its default), since the mixer has one for every sequencer track; an open sets the channels a project leaves out back to their defaults. |
| `instruments` | each track's own instrument (`instruments.rs`) | `mpfree-instruments 1` | A `track <n> <node id> [extras]` line for each track that has its own instrument (numbered from 1; an engine's id such as `mpfree.fm`, or `clap:<plugin id> file=<file>.clap`), then its `param` lines and, for a plugin, its `state` (base64, the rack's last snapshot). Written only while a track has one; no part (New, or an older project) is every track on the kit. |
| `mod` | the mod engine (`modmix.rs`) | `ModMix::save` | The mod engine's set-up (the macros' targets, the matrix) and the macros' names. Loaded after the mixer's inserts are in. |
| `synth` | the synth (`synth.rs`) | `mpfree-synth 1` | `[options]` (the Synth tab's options, as `settings/synth.conf`), then one `[sound <engine>]` section per engine with its sound (`SoundPreset::capture`) and preset name. |
| `sampler` | the kits (`sample_page.rs`) | `mpfree_sampler::save` | The kits, their pads and settings; their sounds go in `samples/` as WAV, named by the sound's id, written only when missing. |
| `midi` | the I/O model (`mpfree-midi-ui`) | `MidiPart` | The I/O setup's song settings (`MidiSetup::project`): tracks' outputs, Tempo from, clock out, CV jobs, learned controls. Each port's input and thru are the rig's, kept for every project, and an open leaves them (`MidiSetup::set_project`). No part (New, or an older project) is the defaults (`Player::open_midi`). |
| `looper` | the looper (`looping.rs`) | `mpfree-looper 1` | Each loop with its settings and its layers' mixes (`mpfree_looper::save`); each layer's audio goes in `samples/` as a WAV named for its content (`loop-<128-bit id>.wav`), written only when missing. |
| `audiotracks` | the audio tracks (`audiotracks.rs`, `mpfree-audiotracks`) | `mpfree-audiotracks 1` | The rate, each track kept (`track=N input= monitor= level= pan= muted= solo=`) with its regions (`region start= end= take= src= gain= fade_in= fade_out=`), and each take's place (`take=N track= group= pass= bpm= anchor= frames= channels= file=`), as `mpfree_audiotracks::save` writes them. Each take's audio goes in `samples/` as a 32-bit float WAV (mono or stereo, as recorded) named for its content (`take-<128-bit id>.wav`, the channels counted in), written only when missing. No part (an older project) leaves the audio tracks as they are. |
| `arp` | the arps (`app/arp/part.rs`) | `mpfree-arp 1` | A `track N` block for the armed track and each track whose arp isn't the default (a track left out opens with the default; a project can have 128 tracks), `key = value` lines: the panel's `mode`, what the arp `plays` while its track isn't armed (what a through-arp track's clips hear), `rate` (also Note Repeat's), every `mpfree_arp::Settings` field (order, octaves, gate, swing, velocity, accent, snap, chord, run, turns, strum, retrigger, repeats, ramp, start), `scale` by name and `root`, `swing-from`, and the pattern as it plays (`pattern`, `Pattern::to_text`) with the library pattern it came from (`pattern-name`). Latch isn't kept. No part (New, or an older project) is every arp at its default. |

The tempo and the transport's swing are in `project.txt` (`tempo`, and `swing = <grid ticks> <amount>` among its extra lines).

Effects are made by id through `project::make_node(id, extras, plugins)`: a built-in one through `mpfree_fx_ui::make`, a plugin (`clap:...`) through the rack, in its own process. A track's instrument is made the same way (`project::instruments_load`: an engine at its defaults through `synth::engine_node`, then its `param` lines). A node keeps more than its parameters with `Node::save_state` / `load_state` and names what it needs to be made again (a plugin's file) with `save_extras`. A plugin's saved state is the rack's last snapshot (taken when it starts and every few seconds, `SandboxNode::saved_state`), so a save never waits on its process; its parameters are saved apart and applied after the state, so a knob turned since the snapshot is still kept. An effect or instrument this build can't make (a plugin whose file isn't in `files/plugins/` is named by its file) is left out of the open with a line in the log; the rest of the chain opens, and the track plays the kit.

### Projects in the pad player

- **Save** (SHIFT + BROWSE on any page, or Save in the browser). The audio loop first has the looper read its loops as they play (`LoopHost::save`: a few periods, more while a loop records, with no glitch) and the audio tracks read their takes (`AudioHost::save`: the worker makes room, then the takes are copied a piece a period; it waits while a track records), then copies its engines' parts between periods (`project::Collected`: the tempo and swing, a copy of the clips, the mixer's and mod engine's text, the synth's, the tracks' instruments, the loops and the audio tracks with their audio) and sends them to the screen thread (`ToUi::ProjectParts`), which adds its own (the kits with their sounds, the I/O setup) and gives the lot to the browser's worker (`Browser::save_with`). The worker writes it whole or not at all. While an open is still landing, a save is refused with a line in the log. The parts wait on the audio thread until the screen has room for them (the loops' and takes' audio is never freed there). An open arriving while a save is under way stops the save (`LoopHost::cancel_save`, `AudioHost::cancel_save`; what was read goes to their workers to drop). The screen names each take by hashing its audio (`project::audio_part`, as the loops' layers are) and the library's worker writes each take's WAV a piece at a time (`mpfree_audiotracks::save::write_wav`). The browser is stepped while its worker is busy, so a save made from another page lands and is said.
- **Open** (a project's row, Open or an earlier save). The screen thread reads the parts and builds what is costly there (`project::opening`: the time engine compiled from the clips, the mixer's effects and the tracks' instruments made and loaded, a plugin's process started and given its state), opens the kits itself (`SamplePage::open_part`) and the I/O setup, then sends one box to the audio loop (`FromUi::OpenProject`). The loop stops the transport and every sound, sets the tempo, swaps the time engine in, gives the synth its sounds and options, puts each track's instrument in (a track the project doesn't name is emptied and plays the kit; the old instruments ring out), and opens the mixer over the next periods (`MixHost::open` / `tend_open`: every insert out, the new ones in order, then the strips' controls, the pad mutes and the mod engine), using the mixer's queued edits so no period allocates. The engines it replaced go back to the screen thread in the same box to be freed (`ToUi::ProjectBack`). The looper's layers are read on a thread of their own (`project::loops_open`) and follow (`FromUi::OpenLoops`); meanwhile every loop fades out as Clear does (`LoopHost::clear_all`), and the project's loops go in once all are empty (`LoopHost::tend_open`). The audio tracks are the same: their takes are read on a thread (`project::audio_open`; a take whose file is missing or isn't its sound is left out with the regions playing it, and said) and follow (`FromUi::OpenAudio`); meanwhile every recording ends, and once nothing records the tracks are emptied, every take forgotten (`TrackHost::clear`, `AudioTracks::clear`); then the part goes in (`TrackHost::open`, `AudioHost::load`: the takes copy in over the next periods), and the screen side's copy of each track's settings is read back from the tracks. A part that doesn't read is left out with a line in the log; its engine keeps what it has.
- **New project.** Once the start-up effects are in, the audio loop sends its parts as they are at start, once (`ToUi::ProjectDefaults`). New project opens a project made from them with an empty song of 8 tracks and straight swing (`project::new_snapshot`), the loops and the audio tracks are cleared, every arp goes back to its default (no `arp` part), the I/O setup's song settings go back to theirs (the rig's stay), every track plays the kit (no `instruments` part), the Synth tab's options are the defaults, and the kits go back to the factory kits loaded at start (`SamplePage::reset_kits`).
- **At start** (Prefs, `Settings::open_last`, kept as `at-start`). The browser keeps the project last saved or opened with Recent and Favourites (`project = Projects/<name>` in `browser/lists.txt`; `Browser::last_project`; a project moved to the Trash is forgotten). With Last project picked, the screen opens it once, when the start's parts have come (`Player::open_at_start` in its tick, through `Browser::open_last`, which leaves the Pads tab's picker open); if the library can't open it, `BrowseAction::LastNotOpened` says why in the log and the new project stays.
- **Unsaved work.** The screen marks the work unsaved when it sends an edit (`Player::saw_action`: anything but pads played, pages, kits picked, the Level, the transport but its swing, previews, the UNDO light and list, and the browser's own; a kit's view sent to the audio thread counts only when the Sample page's edit count moved, so an open's kits still loading after the settle time on a slow machine, or a new tempo's stretched copies, are not edits) or an engine reports one (clips, steps, synth, chain and macros, and `ToUi::Edited` when the app sees a Synth tab option or an arp change made with the controls: `App::take_project_edit`, which compares the arps by a hash a few times a second without allocating), except in the moments after an open or New lands (that is the open's own news). It errs on the side of asking: a clip launched counts too, since the time engine reports launches with its edits.
- **The project's and the system's** (Jay, 13 Nov). The project keeps what makes the song sound as it does: the parts above, the tempo and swing, the Synth tab's options and the arps. Prefs keep what belongs to the player, the MPC and the gear around it, the same in every project, which New leaves alone: the Prefs rows, the found buttons, the metronome, the Looper's level and measured latency, touch tuning, the MIDI rig (port inputs, names, delays) and CV tuning, Recent and Favourites, the pattern library. The Level and the line input never come from a project. A template for New project comes later.
- **Autosave** (Jay's pick, 14 Nov: one Autosave project). `Player::autosave_tick` counts `gui::AUTOSAVE_S` (120 s) while the work is unsaved and has been edited since autosave last asked (`autosave_dirty`, set by `mark_unsaved`), and not while a project opens or before At start was looked at. Then it asks the audio loop for its parts as a save does (`FromUi::SaveProject`), marked as autosave's (`autosaving`); `Player::save_project` hands them to `Browser::autosave_with`, which names the snapshot `Autosave of <project>` and sends `Job::Autosave`. The worker marks `project.txt` (`autosave = yes`, `Meta::mark_autosave`) and saves into `Library::autosave_project` (`Projects/Autosave`, or `Autosave 2` ... when the name is a project of the musician's, unmarked) keeping no earlier saves. `Outcome::Autosaved` changes nothing else: the current project, the unsaved mark and the status line stay, and the app logs `project: autosaved into Autosave` to the terminal only. A save the musician asks for meanwhile wins (`Player::ask_save` clears the autosave's mark, so the parts serve the save); parts asked for that never come (an open stopped the save) are given up after the same time and asked for again. A project whose `project.txt` is marked opens as unsaved work (`Browser::open_landed` keeps the mark; the status says when it was autosaved), with no project, so SHIFT + BROWSE saves it as a new one.
- **Setting the last run's work aside.** The app's only way out is the power switch (or a crash), so autosave's project can hold the last run's unsaved work. Before anything autosaves, `Player::open_at_start` calls `Browser::keep_newer_autosave`: when autosave's `saved` is newer than the last project's (`browser/lists.txt`'s `project`, 0 if none), `Library::keep_autosave` renames it `Unsaved <date> <time>` (when it was written; ` 2` ... if taken), so this run's autosave starts a new one. It stays marked, so it opens as unsaved work. The hint line and the Events log say where it went (`Unsaved work from the last run is in Projects > ...`, or the safe start's line after a crash, `crash::screen_line`). If the rename fails, autosave is off for the run, with a line in the log, rather than write over it. Known limits: autosave's `samples/` keeps every sound it was ever given until the project is set aside (sounds no part names are never removed, as in any project); a project set aside stays until the musician deletes it.
- **Audio tracks and the work folder.** The audio tracks keep their takes in RAM today (streaming waits for the stick speed test), so their part's WAVs are written from memory and `Library::work_dir("audiotracks")` is not used yet. When takes stream from the stick, they will live in that folder while unsaved, a save will copy them in with `NewSample::file`, and New project will call `clear_work("audiotracks")`.

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

`BrowseAction`s for the app: `LoadSample` / `LoadToPad` / `LoadKit` / `LoadFactoryKit` (to the sampler thread's loader), `LoadPreset` (with the engine's folder name, to the synth thread's `mpfree-instruments`), `TryPreset` / `RevertPreset` / `PickerClosed` (pickers), `Audition` / `StopAudition` (the preview path), `ProjectOpened` (give each engine its part), `ProjectSaved`, `SaveRequested` (Save as got a name: call `save`), `SaveLists` (write it to the store as `files/browser/lists.txt`), `ExportAsked` / `ExportStart` / `ExportCancel` (the Export panel: [export.md](export.md#the-apps-part)).

`mpfree-library` and `mpfree-browser-ui` are linked into the pad player, so `tools/mpc-one/tests/test_app_mpc.py` reads them (`LINKED_SRC`) and lists the library's write sites, all in `lib.rs` and `project.rs` and all under the library folder, line by line (`LIBRARY_NEEDLES`, `LIBRARY_OPENS`). A change to one of them changes that list in the same pull request.

**In the pad player (the Pads tab).** With `--data`, the app sends the screen thread the folder and the saved lists (`ToUi::Library`); the screen opens `Library::at` (which makes `library/` and its place folders when missing) and a `Browser` on the library's worker thread, opened with `Pick::Pads`. `mpfree_browser_ui::step` runs each tick while Browse shows and after each touch. Its actions: `LoadSample` and `LoadToPad` go to `SamplePage::load_file`; `Audition` reads the file on a thread (`sample_page::read_in_thread`) and hands the sound to the audio thread in a box (`FromUi::Preview`, `mpfree-app/src/preview.rs`), which mixes it into the main out at the preview level and gives the box back to be freed on the screen thread (`ToUi::PreviewBack`); `StopAudition` fades it out over 256 frames (`FromUi::PreviewStop`); `SaveLists` goes to the store (`files/browser/lists.txt`). The others are logged as not wired on the Pads tab. Without `--data`, the Browse view shows a card that says so.

## Not yet

- The other pickers in the pad player (presets on the Synth tab). The Pads tab's picker and the full browser are wired (above).
- `LoadKit` and `LoadFactoryKit`, and `LoadPreset` / `TryPreset` in the full browser: a kit file's format and saving a kit on its own come later (sampler thread); presets with the synth thread's picker.
- The arps' and the audio tracks' parts (above).
- Sharing the library over Ethernet (WebDAV and a web page), a desktop `mpfree-sync`, and USB file mode: plan section 9, decisions 9 to 11.
