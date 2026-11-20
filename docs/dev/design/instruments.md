<!--
covers: rust/mpfree-app/src/instrument_list.rs rust/mpfree-app/src/project.rs rust/mpfree-app/src/app/pads_preview.rs
checked: 2026-11-20 c5b97009
-->
# Instruments apart from tracks

**Status:** step 1 on main (Play on the pads, #722, train 124, 20 Nov 2026); step 2a built (the shared synth as instrument 1 of the project's list, project format 2, `rust/mpfree-app/src/instrument_list.rs`, 20 Nov 2026). Jay approved the plan and its five answers on 20 Nov 2026 (11:16 UTC). The plan, its rules and the reviews it went through: "Instruments, pads and tracks: the redesign" (https://claude.ai/artifact/A2iE1bDqPxk2uSJABCgTJg). Nothing here has run on the MPC One yet.

## The model

Jay's rule (20 Nov 2026): **instruments make the sound, tracks organize and record performances, and the pads are controllers that play instruments.** A track is a place you record to. The pads, MIDI and scripts play instruments; tracks record what is played and play it back to instruments; every instrument's sound goes to the mixer, recorded or not. So the code names and routes by instrument, never by track, and a track's mixer channel is not an instrument's output.

Today an instrument belongs to a track: a track plays a kit, the shared synth (the Synth kit) or its own instrument (`instruments.rs`), and the pads play the armed track's. The redesign takes that apart in steps, each of which ships on its own, keeps old projects playing as they did, and gives the musician something they couldn't do before:

1. **Try a synth on the pads** (on main): Play on the pads plays the shared synth while the armed track keeps its kit, and REC records it on a track nobody set up ([rec-arm.md](rec-arm.md), `app/pads_preview.rs`).
2. **Instrument identities** (this page, below): instruments get lasting ids in a list in the project. No visible change.
3. Pad focus: the pads play an instrument, not a track.
4. Tracks send to instruments; each knob has one owner at a time.
5. Recording by source, with one recording plan behind both the REC chip and REC.
6. The instrument picker and banks.
7. MIDI and scripts play instruments.
8. Layers, mutes and stems by instrument.
9. Audio points.

## Step 2: instrument identities

Anything that must point at an instrument (a track, later a knob's owner, a script or a MIDI port) needs a name for it that stays put when tracks are added, moved or taken away. An **`InstrumentId`** is that name: a number from 1, which is not a track number, a mixer channel, a slot or a place in a list. A project gives each new instrument the next number, keeps that counter in its file, and never gives a number out again once its instrument is gone, so something that named an old instrument can't reach a new one by mistake.

An instrument's **saved definition** (what it is: its id and kind, later its name and sound) is kept apart from its **running engine** (built, loaded and silent, or sounding). The list holds definitions only; engines are built and freed off the audio thread, and the loop swaps them in, as projects open today.

The step goes in three slices, each proven with the shared Poly before the next:

- **2a (built):** the shared synth becomes instrument 1 of the project's list, and tracks on it point at it by that id. It saves, loads and plays exactly as before.
- **2b:** the tracks' own instruments join the list, each with its own id.
- **2c:** tracks get lasting ids too, so knob owners (step 4) and anything else that must survive moving tracks has something steady to point at.

### 2a as built

- **The list** (`instrument_list::InstrumentList`): the instruments, each an `InstrumentId` and a `Kind`, and the next id. In 2a it always holds one: `Kind::SharedSynth` as `InstrumentId::SHARED_SYNTH` (1), the pads' synth, whichever engine the Synth tab plays. Its sound stays in the `synth` part and it sounds on the Synth strip; its running engine is still `synth::Synth`.
- **Its part**, `instrument-list`, saved with every project:

  ```text
  mpfree-instrument-list 1
  next 2
  instrument 1 shared-synth
  ```

  `InstrumentList::parse` refuses a part with a line it can't read, an id listed twice or not below `next`, or without the shared synth as instrument 1.
- **Tracks point at it by id.** The `track-kits` part went to version 2: `track 4 instrument 1` where version 1 said `track 4 synth` (`sampler::TrackKits::text`). `project::track_kits_parse` reads both versions, each with its own word only, into `TrackPick::Kit` or `TrackPick::Instrument`. The opener turns an instrument the list has into the shared synth's entry; a track whose instrument isn't in the list plays kit 1, and the open's log says which and why (`project::tracks_play`).
- **The project's format.** `project.txt` says `format = 2` (`project::FORMAT_KEY`, `FORMAT`), written by every save and by New project. A project with no format line is format 1, saved before instruments had ids: it has no list, and its shared synth is instrument 1, as in a new project. Nothing is said for that, because nothing was lost. A format newer than this build's, a format line that doesn't read, a format 2 project with no list, or a list that doesn't read are each said in the log, and the project opens with the new project's list. The library's own first line (`mpfree-project 1`) is the folder's format and doesn't change.
- **Threads.** The list is read and made on the screen thread with the rest of the open (`project::opening`, `Opening::instrument_list`). The audio loop swaps it in whole between two periods, and its old one goes back in the box to be freed on the screen thread (`main.rs`, mirrored by the test session). A save copies it out as text with the other parts (`Collected::instrument_list`). The loop never makes or frees one.
- **What an older build does with a format 2 project:** it can't read the track kits part's version 2, says so, and plays every track on kit 1; it keeps the list as a part it doesn't know. Projects are not promised to open in older builds.
- **Tests:** `instrument_list::tests` (the new list, the counter kept, broken lists refused); `project::tests::a_project_from_before_instrument_ids_opens_and_saves_as_it_played`, `a_track_whose_instrument_is_not_listed_plays_kit_1_and_says_so`, `the_project_format_and_the_list_are_written_and_a_problem_is_said`; `sampler::tests::each_track_plays_its_own_entry_and_a_kit_lives_on_its_first_track` (the part's text); and through the whole session, `smoke::song_workflows::tests::a_project_from_before_instrument_ids_opens_saves_and_plays_the_same`: a song saved as the build before wrote it, with track 2 on the synth and track 3 on kit 2, opens from BROWSE with those tracks on them, saves as format 2 with track 2 on instrument 1, and after New project opens again with every track on what it played and the same notes.

What plays is unchanged: the track kits a project opens with are the same as before, and nothing that plays reads the list yet.
