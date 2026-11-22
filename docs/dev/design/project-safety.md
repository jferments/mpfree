# Project safety: opening and saving whole

Phase 0 of the [audio tracks rethink](https://claude.ai/artifact/2rrcXW5CTeSmEuMA6EWwu7), approved by the owner on 22 Nov 2026: the rules that keep a project from being half opened, half saved or saved over with things missing. They come before any change to the project format, because builds already on a stick can't be fixed afterwards.

Code: `rust/mpfree-app/src/project.rs` (`check`, `completed`, `opening`), `rust/mpfree-app/src/gui.rs` (`Player::open_project`, `project_opening`, `open_refused`), `rust/mpfree-browser-ui/src/state.rs` (`Browser::open_refused`). The project's files and parts are in [The browser and projects](browser.md#the-parts).

## Opening is all or nothing (0.1, as built)

MPFree reads the whole project before anything of the open one changes. If any part it needs can't be read, nothing goes to the engines and the project that was open stays exactly as it was.

1. **The gate** (`project::check`) runs first, on the snapshot the library read:
   - **The project format** (`format = N` in `project.txt`; no line is format 1). A format above `project::FORMAT` was saved by a newer MPFree: refused, "it was saved by a newer MPFree (project format 3; this one opens up to 2). Update MPFree to open it".
   - **What it needs** (`needs = <feature> ...` lines in `project.txt`, any number, words separated by spaces). A feature not in `project::KNOWN_NEEDS` is refused, "it needs audio-clips, which this MPFree hasn't got. Update MPFree to open it". This build knows one word, `synths-without-tracks`, written while a project has a synth that no track has (one picked for the pads, the instrument redesign's step 6, [instruments.md](instruments.md#step-6-synths-with-no-track-and-the-list-of-synths)): an older build would read that synth's settings as the last track's instrument's, so it refuses the project instead. Audio tracks' phase 1 writes `needs` with the first save that has an audio track, so a build with this gate refuses such a project instead of opening an audio track as an empty note track. A build adds a word to `KNOWN_NEEDS` when it can open what the word stands for.
   - **The parts read elsewhere**, each by its own reader: the kits (`sampler`, by its first line), the loops (`mpfree_looper::save::from_text`), the audio tracks (`mpfree_audiotracks::save::from_text`), the arps (first line), the list of instruments (`InstrumentList::parse`) and what the tracks play (`project::track_kits_parse`).
2. **The missing parts** (`project::completed`): a project saved before the clips, mixer, modulation or synth part existed gets New project's, so no engine keeps the last project's. Before the start's parts have come (`ToUi::ProjectDefaults`) such a project is refused with "the sound is still starting; try again in a moment". A project without a `looper` or `audiotracks` part opens with no loops or audio tracks (`Opening::clear_loops` and `clear_audio` are always set), and one without a `sampler` part with the factory kits, as New project.
3. **Building** (`project::opening`, which now gives `Result<Opening, String>`): the clips, the mixer, the synth, the tracks' instruments and what the tracks play are read and built on the screen thread, as before. A part that doesn't read ends the open with "its mixer can't be read (...)". Nothing has been sent anywhere yet, and what was built is dropped there.
4. **Only then** does the screen open the kits (`SamplePage::open_part`) and the I/O setup, send the box to the audio loop (`FromUi::OpenProject`) and start the threads that read the loops and the audio tracks.

**Refused.** `Player::open_refused` says "Didn't open Song: it was saved by a newer MPFree (...). Update MPFree to open it. The project you had open is unchanged" on the hint line, in the browser's status line (as a problem) and in the Events log (`project: didn't open ...`). The browser had already taken the project as the one worked on when the library gave it (`Outcome::Project` or `Outcome::Version`), so `Browser::open_refused` puts back what it changed: where SHIFT + BROWSE saves, the project Prefs > At start opens, Recent, the date it was created and the unsaved mark (`BeforeOpen`). Without that, the next save would write the open project over the refused one. The browser forgets what it kept once the open lands (`Browser::open_landed`).

**Not refused** (opens as before, and is said): lines a part doesn't know (a newer build's, skipped by every reader), and things this build can't make, such as a plugin whose file isn't on the stick, an effect it hasn't got or a track pointing at an instrument the list hasn't got. A format 2 project with no list of instruments opens with New's list, said. Lenient parts stay lenient: the Perform set skips a line it can't read, the MIDI setup and the arps' settings say what they left out, and a pads part that doesn't read (what the pads play is how you play, not the song) is said and the pads follow the track.

**Still to come in phase 0:** a project that opened with things left out is saved over only after MPFree says what would be lost; every background job (a file read, a fitted copy, an import) carries the project's generation, so a result for a project no longer open is dropped (today the loops' and audio tracks' read threads already are, by dropping their channels on each open); and "every recording found" waits for the shared streaming layer.

**Tests:** `project::tests::a_project_this_build_cant_read_whole_doesnt_open_and_says_why` (each part, the needs line, the missing parts), `the_project_format_and_the_list_are_written_and_a_problem_is_said`, and in the screen `gui::tests::a_project_this_build_cant_read_doesnt_open_and_the_open_one_stays`, which opens a project whose clips are `mpfree-clips 9` (as an `mpfree-clips 2` project is to a build from before it) and one of a newer format, and checks that nothing reaches the engines and the browser still saves to the project that was open.

## Saving (0.2 and 0.3, planned)

- The audio tracks' save waits until every take has settled (no recording and no tail) before it copies anything, and builds its copy off the sound thread.
- Every save commits in order: the media (samples, takes, loops) written and synced first, then the project file through the library's temporary file, sync, rename and directory sync, so a cut-off save never leaves a project naming a file that isn't there.

## Tests under pressure (0.4, planned)

Generated sequences of operations (a seeded generator, no new crate), injected faults (a slow read, a write error, a missing file, a result after a project switch, a save stopped between steps) and the allocation check across whole workflows (saving, opening, starting and stopping recordings).
