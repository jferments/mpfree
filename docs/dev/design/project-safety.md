# Project safety: opening and saving whole

Phase 0 of the [audio tracks rethink](https://claude.ai/artifact/2rrcXW5CTeSmEuMA6EWwu7), approved by the owner on 22 Nov 2026: the rules that keep a project from being half opened, half saved or saved over with things missing. They come before any change to the project format, because builds already on a stick can't be fixed afterwards.

Code: `rust/mpfree-app/src/project.rs` (`check`, `completed`, `opening`), `rust/mpfree-app/src/gui.rs` (`Player::open_project`, `project_opening`, `open_refused`, `left_out`), `rust/mpfree-browser-ui/src/state.rs` (`Browser::open_refused`, `left_out`, `save_with`). The project's files and parts are in [The browser and projects](browser.md#the-parts).

## Opening is all or nothing (0.1, as built)

MPFree reads the whole project before anything of the open one changes. If any part it needs can't be read, nothing goes to the engines and the project that was open stays exactly as it was.

1. **The gate** (`project::check`) runs first, on the snapshot the library read:
   - **The project format** (`format = N` in `project.txt`; no line is format 1). A format above `project::FORMAT` (3) was saved by a newer MPFree: refused, "it was saved by a newer MPFree (project format 4; this one opens up to 3). Update MPFree to open it". Since the sampler refactor's step 14 (23 Nov 2026), a format below it, or none, is refused too, and not converted: "it was saved by an older MPFree, which this one doesn't open (project format 2; this one opens 3)". There are no serious projects yet (the owner, 22 Nov 2026), so older projects aren't carried forward ([sampler-architecture.md](sampler-architecture.md#kits-in-the-list-of-instruments-step-14)).
   - **What it needs** (`needs = <feature> ...` lines in `project.txt`, any number, words separated by spaces). A feature not in `project::KNOWN_NEEDS` is refused, "it needs audio-clips, which this MPFree hasn't got. Update MPFree to open it". This build knows four words. `synths-without-tracks` is written while a project has a synth that no track has (one picked for the pads, the instrument redesign's step 6, [instruments.md](instruments.md#step-6-synths-with-no-track-and-the-list-of-synths)): an older build would read that synth's settings as the last track's instrument's, so it refuses the project instead. `audio-tracks` is written while a track in the track list is an audio track (audio tracks' phase 1a, [audio-tracks.md](audio-tracks.md#in-the-one-track-list-phase-1a)): a build with this gate refuses such a project instead of opening an audio track as an empty note track. A project with both says `needs = synths-without-tracks audio-tracks`. `linked-song-list` is written while the song list is linked to the song, or a marker carries its section's scene or passes (the Arrange refactor's phase 4b, [arrange.md](arrange.md#scenes-and-the-song)): an older build would show the list empty and lose them. `free-loops` is written while the looper's part holds a free loop (Sync: Free, [looper.md](looper.md#free-loops)): an older build would skip its `clock=free` and the `free-clock` line and play it as a song loop, in the song's time and at the song's tempo. A build adds a word to `KNOWN_NEEDS` when it can open what the word stands for.
   - **The parts read elsewhere**, each by its own reader: the kits (`sampler`, by its first line), the loops (`mpfree_looper::save::from_text`), the audio tracks (`mpfree_audiotracks::save::from_text`), the arps (first line), the list of instruments (`InstrumentList::parse`) and what the tracks play (`project::track_kits_parse`).
2. **The missing parts** (`project::completed`): a project saved before the clips, mixer, modulation or synth part existed gets New project's, so no engine keeps the last project's. Before the start's parts have come (`ToUi::ProjectDefaults`) such a project is refused with "the sound is still starting; try again in a moment". A project without a `looper` or `audiotracks` part opens with no loops or audio tracks (`Opening::clear_loops` and `clear_audio` are always set), and one without a `sampler` part with the factory kits, as New project.
3. **Building** (`project::opening`, which now gives `Result<Opening, String>`): the clips, the mixer, the synth, the tracks' instruments and what the tracks play are read and built on the screen thread, as before. A part that doesn't read ends the open with "its mixer can't be read (...)". Nothing has been sent anywhere yet, and what was built is dropped there.
4. **Only then** does the screen open the kits (`SamplePage::open_part`) and the I/O setup, send the box to the audio loop (`FromUi::OpenProject`) and start the threads that read the loops and the audio tracks.

**Refused.** `Player::open_refused` says "Didn't open Song: it was saved by a newer MPFree (...). Update MPFree to open it. The project you had open is unchanged" on the hint line, in the browser's status line (as a problem) and in the Events log (`project: didn't open ...`). The browser had already taken the project as the one worked on when the library gave it (`Outcome::Project` or `Outcome::Version`), so `Browser::open_refused` puts back what it changed: where SHIFT + BROWSE saves, the project Prefs > At start opens, Recent, the date it was created and the unsaved mark (`BeforeOpen`). Without that, the next save would write the open project over the refused one. The browser forgets what it kept once the open lands (`Browser::open_landed`).

**Not refused** (opens as before, and is said): lines a part doesn't know (a newer build's, skipped by every reader), and things this build can't make, such as a plugin whose file isn't on the stick, an effect it hasn't got or a track pointing at an instrument the list hasn't got. A project with no list of instruments (an MPC program brought in by `mpfree-mpc-import` has only its kit) opens with New's list; anything in it that named an instrument by id is said where it is read (a track's pick, a track's own instrument). Lenient parts stay lenient: the Perform set skips a line it can't read, the MIDI setup and the arps' settings say what they left out, and a pads part that doesn't read (what the pads play is how you play, not the song) is said and the pads follow the track.

**Still to come:** "every recording found" waits for the shared streaming layer (the audio tracks' phase 1 builds recordings on `mpfree-stream`, agreed with the streaming thread on 22 Nov). Late results are 0.3, below; what happens to a project that opened with things left out is [Never saved over](#a-project-that-opened-with-things-left-out-is-never-saved-over-as-built), below.

**Tests:** `project::tests::a_project_this_build_cant_read_whole_doesnt_open_and_says_why` (each part, the needs line, the missing parts), `a_project_with_an_audio_track_says_it_needs_audio_tracks`, `the_project_format_and_the_list_are_written_and_a_problem_is_said`, and in the screen `gui::tests::a_project_this_build_cant_read_doesnt_open_and_the_open_one_stays`, which opens a project whose clips are `mpfree-clips 9` (as an `mpfree-clips 2` project is to a build from before it) and one of a newer format, and checks that nothing reaches the engines and the browser still saves to the project that was open.

## The engines an open replaced go back (live shows A2, as built)

An open swaps the new project's engines in on the audio loop and sends the ones it replaced back to the screen thread in the same box, to be freed there: freeing a time engine or a mixer can take far longer than an audio period. Before, the audio loop kept one box at a time (`project_back`) while the screen's channel was full, and a second open arriving then freed the older box on the audio thread.

- The audio loop keeps **two slots** (`gui::OPENS_ON_THEIR_WAY`) and sends each box on as soon as the screen's channel has room.
- The screen counts the opens it has sent whose box hasn't come back (`Player::opens_out`, down by one on each `ToUi::ProjectBack`). With two on their way, an open or New project is refused before anything changes, through the usual refusal (`open_refused`): "Didn't open Song: the last two projects are still going in; try again in a moment. The project you had open is unchanged". It can only happen with opens a few milliseconds apart while the screen is busy.
- So a box always finds a slot. If one ever didn't (a bug), a debug build stops on an assertion and a release build frees it as before, with a line in the Events log saying it wasn't expected.

This is the first step of stage A of the [live shows plan](live-shows.md): a prepared next song will go back the same way, with its slot reserved before it can go.

**Test:** `gui::tests::opens_wait_for_the_engines_they_replaced_to_come_back` (two opens go, a third and New project wait and say so, the next goes once a box is back).

## Saving

### A project that opened with things left out is never saved over (as built)

The plan said such a project "is saved over only after MPFree says what would be lost". As built it is never saved over by a plain save: SHIFT + BROWSE, the browser's Save and the power-off dialog's save all go through one path with no room for a question (the power-off one least of all), so the save goes into a new project beside it and says why. Nothing is lost and nothing waits on an answer; the musician can still save over it on purpose with Save as.

**What counts as left out:** what the project names that a save would drop, never a note that changes nothing saved. Each source gives a few words for the musician, as it finds out:

| Where | Left out | Code |
|---|---|---|
| The mixer (screen thread, at the open) | an effect this stick can't make; an effect that didn't take its settings | `mixer_load` → `MixLoad::left_out` |
| The tracks' instruments (same) | an instrument that can't be made, one past the most at once or on no track, its settings, a Wavetable sound that didn't move to Poly, a sound's effects and macros | `instruments_read`, `design_in` |
| What the tracks play (same) | a track pointing at an instrument the list hasn't got | `tracks_play` |
| The kits (read on a thread) | each sound, chop, pad, take or scene the sampler left out (missing, or no room), or the whole kits part | `SamplePage::take_left_out` |
| The loops (read on a thread) | each loop whose sound can't be read, or the whole part | `loops_open` (third list) |
| The audio tracks (read on a thread) | each take whose file can't be read or isn't the take's, or the whole part | `audio_open` (third list) |
| The audio tracks (audio loop) | takes or tracks `AudioHost::load` had no room for | `TrackHost::take_left_out` → `ToUi::LeftOut` |

`project::opening` collects the first three in `Opening::left_out`; `Player::open_project` hands them to the browser before the box goes to the audio loop, and the rest follow through `Player::left_out` as they come. Not left out: a newer build's lines (the format and `needs` gate are for that), a lasting id given anew, takes recorded at another rate (converted to the rate mpfree runs at, all there), sounds converted to the sound's rate. The looper's own load can only keep a loop out if the loop wasn't empty, which the clear before it rules out; streaming's open errors (`AudioHost::open_errors`) join when the app streams.

**The browser** (`Browser::left_out`, `rust/mpfree-browser-ui/src/state.rs`) keeps the list for the project open. `save_with` then saves into `Library::copy_project_name` (`Night drive 2`, the next free number), which becomes the project when the save lands, with the status line and hint line saying *Saved as Night drive 2: Night drive opened without the effect X on Master and 2 more, so it stays as it was* (the Events log has each line from the open). The list is cleared when a save lands (the new project holds all it has), when another project opens (an open takes its own list; a refused open puts back the list of the project still open, `BeforeOpen`), for New project and for Save as: a typed name is the musician's choice, even the project's own, and Save as offers the name beside it first. Autosave never saves over a project anyway, and its project opens as new, unsaved work.

**A save waits for the kits.** Until the opened project's kits are read, the sampler still holds the last project's, so a save then would have put the last project's kits into this one. A save now waits (`SamplePage::opening`, "not saved, the kits are still opening"), as it already did for the loops and the audio tracks, and so does autosave.

**Tests:** `tests/browser.rs` `a_project_whose_open_left_things_out_is_saved_beside_it` (the save beside, the original's parts and versions untouched, later saves into the new project, Save as over the original, a refused open, New), `gui::tests::a_project_that_opened_with_things_left_out_is_saved_beside_it` (a missing plugin effect, kit sounds whose files aren't there, a take the audio loop had no room for, the save refused while the kits are read), and the left-out lists in `project::tests` (`an_effect_this_build_cannot_make_is_left_out_and_said`, the loops' and takes' open tests).

### The audio tracks' save waits and never allocates on the sound thread (0.2, as built)

`AudioHost::save` used to build the save on the audio thread: `AudioTracks::save_file` (a list of takes, a list of tracks, each track's regions copied) and, streaming, each take's file name cloned. Streaming, it even built it every period before checking whether the takes had settled, and threw it away until they had.

Now (`rust/mpfree-audiotracks/src/host.rs`, `tend_save`):

1. `save` asks the worker for a room (`ToWorker::Room`: `Saved::room`, every track with room for all its regions and every take with room for a file name, about 120 KB) and waits for it (`Saving::Asking`).
2. With the room in hand (`Saving::Waiting`), each period checks first that everything has settled: no track records or has a recording's tail still coming, no take is loading and, streaming, the disk has nothing waiting, under way or still to write (`AudioTracks::settled`, `unwritten`, `loading`). Only then does `AudioTracks::save_into` fill the room, without allocating; streaming, each take's file name is copied into its room.
3. The filled room goes back to the worker (`ToWorker::Size`), which trims it to what it holds and, unless streaming, makes room for every take's audio. The audio is then read a piece a period, as before; streaming, the audio is in the take files already and the save is done.

A save cancelled while the worker has the room (an open arriving) has the worker's answer dropped when it comes (`cancelled`), as before. `tests/no_alloc.rs` (`a_save_fills_its_room_without_allocating`) checks that filling a room, settled or not, allocates nothing and gives what `save_file` gives.

## Late results never reach the wrong project (0.3, as built)

A job started for one project can finish after another has opened. Where each kind is caught:

- **The screen's reads** (the kits, the loops, the audio tracks): each open replaces the channel the last open's thread answers on (`SamplePage::opening`, `Player::loops_load`, `Player::audio_load`), so an answer for an older open has nowhere to go; New project's factory kits drop a kit read still under way (`SamplePage::reset_kits`).
- **What the audio loop holds for an open:** the looper's loops waiting to go in (`LoopHost::clear_all` drops them) and the audio tracks' part (`TrackHost::clear` drops it).
- **The audio tracks' background jobs** (new): `AudioHost::clear` empties the tracks for an open or New, and adds one to the host's generation. A fit's buffer and its stretched take carry the generation they were asked for in, and one for emptied tracks is dropped as it comes and counted (`AudioHost::stale`). Before, a fit under way when a project opened came back after the switch and was swapped into the new project's take with the same id (take ids come back after a clear). (The old streaming's disk thread, which needed the same for a fit's reads, was removed in the audio tracks' phase 3a; a generation on the disk's own messages is `mpfree-stream`'s, agreed with its thread.) A sound being brought in goes with the rest.
- **The looper's prints** check the loop's version before they are used (`Printer`, `Looper::loop_status`). Loops become clips of audio tracks in phase 4, which moves them onto the same generation.

**Tests:** `tests/host.rs` `a_fit_from_before_a_project_opened_never_reaches_the_new_takes` (the clear at every stage of a fit; without the generation the old take lands as the new project's take 1).

## Saves committed in order (0.3: already so, checked)

The library already commits a project save in order (`Library::save_project_with`, `rust/mpfree-library/src/project.rs`): the new sounds (including an audio track's take files, copied from its work folder) and the new parts are written beside the project and synced together; then the sounds are renamed into `samples/` and the folder synced; then the old parts go into `versions/`, the new parts into place, the folder synced, and last `project.txt` through a `.part` file, synced, renamed and the folder synced. A save cut off at any point opens as the save before or the new one, never a project naming a file that isn't there (`Library::recover`). The audio tracks hand their part over only once every take has settled (0.2), so the take files a save copies are whole.

## Tests under pressure (0.4, as built)

**Generated sequences** (`rust/mpfree-audiotracks/tests/sequences.rs`): a seeded generator (xorshift, no test crate, the same steps on every machine) drives the audio tracks in their host through 40 steps a seed, 20 seeds: recording 1 or 2 bars on any track (mono or stereo), edits (comp, split, erase, move, gain) wherever they land, undo and redo, a fit to another tempo left running, saves (one in three stopped part way, as an open stops one), opens (New, or the last save, while a fit or save may be under way), and forgetting takes. After every step every region plays a take that is there and a track's regions are in order and never overlap; every save has each take's audio whole and names only takes it holds, and opens in a fresh host exactly as saved; at the end a save reopened in place gives back the same tracks. Six more seeds run with 3 seconds of memory, so recordings and fits are refused or cut short. With the generation of 0.3 taken out, seed 13 fails: two stretched takes from before an open turn up in the next project.

**Injected faults**, and where each is tested:

| Fault | Test |
|---|---|
| A result arriving after a project switch; a stretch that finishes after it was cancelled | `tests/host.rs` `a_fit_from_before_a_project_opened_never_reaches_the_new_takes`, and the sequences |
| A save stopped between steps | the sequences; `tests/host.rs` `a_cancelled_save_leaves_nothing_behind_and_the_next_save_works`; the library's `a_save_cut_off_at_any_step_opens_whole` |
| A write error (a full disk) | the library's `a_sound_that_cannot_be_written_leaves_the_last_save_standing` |
| A missing file | `project::tests::audio_takes_keep_the_names_the_tracks_gave_and_a_missing_one_is_said`; a take's folder gone as its track arms, `tests/files.rs` `a_track_whose_files_cant_be_made_doesnt_arm_and_says_why` |
| A slow read (the disk stalling past the read-ahead) | `mpfree-stream`'s own tests (`tests/plan.rs`); the audio tracks play from memory until phase 3b |
| The app killed mid-take | `tests/files.rs` `a_take_cut_off_is_recovered_at_the_next_start_up_to_its_last_checkpoint` |
| Memory that can't be reserved | `generated_sequences_with_the_memory_full_lose_nothing_else` |
| A part this build can't read | phase 0.1's tests, above |

The streaming layer's own faults (writes and finishes that fail, short reads) come with its P0.5 (`DiskStats` fault knobs); once that is on main, a save with a take at risk ends with a plain reason instead of waiting.

**The allocation check across workflows:** `tests/no_alloc.rs` watches the tracks' calls through recording, punch, loops, locates, tempo changes, comping, every edit, undo and redo, forgetting, a full memory, loading a take from a save, reading takes for one, emptying the tracks for an open (`clear`) and, new, filling a save's room settled or not (`save_into`). The host's own sends to its worker and the disk thread go through unbounded channels, which allocate now and then; making them bounded is the streaming layer's P1 (agreed), after which the check moves up to the host's whole period.
