<!--
covers: rust/mpfree-app/src/instrument_list.rs rust/mpfree-app/src/project.rs rust/mpfree-app/src/app/pads_focus.rs rust/mpfree-seq/src/lib.rs rust/mpfree-seq/src/save.rs
checked: 2026-11-20 c5b97009
-->
# Instruments apart from tracks

**Status:** step 1 on main (Play on the pads, #722, train 124, 20 Nov 2026); step 2a built (the shared synth as instrument 1 of the project's list, project format 2, `rust/mpfree-app/src/instrument_list.rs`, 20 Nov 2026); step 2b built (the tracks' own instruments in the list, 20 Nov 2026); step 2c built (tracks get lasting ids, `rust/mpfree-seq`, 20 Nov 2026); step 3 built (pad focus: the synth chosen for the pads stays whichever track is selected, saved with the project, `app/pads_focus.rs`, 20 Nov 2026). Jay approved the plan and its five answers on 20 Nov 2026 (11:16 UTC). The plan, its rules and the reviews it went through: "Instruments, pads and tracks: the redesign" (https://claude.ai/artifact/A2iE1bDqPxk2uSJABCgTJg). Nothing here has run on the MPC One yet.

## The model

Jay's rule (20 Nov 2026): **instruments make the sound, tracks organize and record performances, and the pads are controllers that play instruments.** A track is a place you record to. The pads, MIDI and scripts play instruments; tracks record what is played and play it back to instruments; every instrument's sound goes to the mixer, recorded or not. So the code names and routes by instrument, never by track, and a track's mixer channel is not an instrument's output.

Today an instrument belongs to a track: a track plays a kit, the shared synth (the Synth kit) or its own instrument (`instruments.rs`), and the pads play the armed track's. The redesign takes that apart in steps, each of which ships on its own, keeps old projects playing as they did, and gives the musician something they couldn't do before:

1. **Try a synth on the pads** (on main): Play on the pads plays the shared synth while the armed track keeps its kit, and REC records it on a track nobody set up ([rec-arm.md](rec-arm.md), `app/pads_focus.rs`).
2. **Instrument identities** (this page, below): instruments get lasting ids in a list in the project. No visible change.
3. **Pad focus** (built, below): the pads follow the selected track or play an instrument chosen for them, which stays whichever track is selected.
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
- **2b (built):** the tracks' own instruments join the list, each with its own id.
- **2c (built):** tracks get lasting ids too, so knob owners (step 4) and anything else that must survive moving tracks has something steady to point at.

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

### 2b as built

- **When an own instrument has an id.** A track's own instrument (`instruments.rs`) gets the next id when it goes into a track that had none, keeps it while the track has one (a new sound or preset loaded in its place is the same instrument, as the slot it plays in is), and loses it for good when it is taken out: a new one gets a new id, never an old one. `Kind::Own { track }` says whose it is.
- **Fixed storage, so the audio loop can follow.** The list keeps an id for each of the 128 tracks (`own`) and the counter; the shared synth is always instrument 1 and needs no entry. Each period the loop calls `InstrumentList::follow(Instruments::held_tracks())`, a bit for each track that holds one (not one only ringing out an old one), and the list gives and drops ids from that alone. It never allocates, so whatever puts an instrument in or takes it out (the Synth tab's Instrument row, the builder landing one, an open) needs no change.
- **Its part** names each one's track, by id:

  ```text
  mpfree-instrument-list 1
  next 4
  instrument 1 shared-synth
  instrument 2 own track 2
  instrument 3 own track 4
  ```

  `InstrumentList::parse` also refuses a track with two, a track out of range, or instrument 1 as an own one. An older (2a) build refuses a list with own lines and says so; the instruments themselves still open there, since their sounds are where they were.
- **Their sounds stay where they were**, in the instruments part, named by track (`track 2 mpfree.fm`): a track has one at most, so the list's track tells which block is which instrument. When step 4 lets tracks send to instruments, the blocks move to ids.
- **On open** (`project::own_ids`): every track the instruments part gave an instrument keeps the id the list gives it, or gets the next one. In a project with no format line (saved before instruments had ids) that is every one, in track order, and nothing is said. In a newer one, a track's instrument with no id in the list, and an id whose track has no instrument (not in the part, or it couldn't be made), are each said, and the list is put right. A track in the track kits part pointed at a track's own instrument plays kit 1, said: tracks can't point at another track's instrument until step 4.
- **Tests:** `instrument_list::tests::a_tracks_own_instrument_keeps_its_id_and_ids_are_never_given_twice`, `the_next_id_is_kept_through_the_text`; `instruments::tests::the_list_of_instruments_follows_the_tracks_own` (with real instruments: a new sound keeps the id, out and in again is a new id); `project::tests::the_tracks_own_instruments_get_ids_that_come_back` (an old project's FM on track 2 and Poly on track 4 become instruments 2 and 3, saved and opened again), `a_list_that_disagrees_with_the_instruments_part_is_put_right_and_said`.

### 2c as built

- **`mpfree_seq::TrackId`** on every `Track` (`Track::id`), and `Project::next_track`, the id the next track gets. `TrackId::NONE` (0) is a track with none yet: one just made with `Track::new`, or read from a project saved before tracks had ids.
- **Given in one place each.** `Project::give_track_id` gives a track going in the next id (or keeps its own if no other track has it), never allocating: `Takes::add_track_as` (ADD TRACK, Rec's new track, a Loop track) and `loops::place` (a Loop track a recording makes). `Project::give_track_ids` gives every track without one, or sharing an earlier track's, the next id in track order: `TimeHost::with_project` calls it, so every track in the time engine has one. The take keeps the track with its id, so UNDO takes it away and REDO brings back the same track; a track added after the UNDO gets the next id, never its.
- **Saved in the clips part:** `nexttrack <n>` before the tracks and `id <n>` after each track's name. An older build skips both lines. A project without them gets 1, 2, 3 ... as it opens, saying nothing; one that has `nexttrack` but a track without an id, or two sharing one, gets new ones for those, said (`project::opening`).
- **Nothing reads them yet.** Tracks can't be moved or deleted from the middle today (UNDO takes away only the last added), so a track's place still names it, and the list of instruments, the track kits and the instruments part keep naming tracks by place. Step 4's knob owners are the first to use the ids (their priority), and anything that lets tracks move will switch those parts to ids as it goes in. `Project::track_with(id)` finds a track by its id.
- **Tests:** `mpfree_seq` `track_id_tests::tracks_get_lasting_ids_that_are_never_given_twice`, `save::track_id_tests::track_ids_and_the_next_id_come_back`; `mpfree_play` `takes::tests::an_added_track_has_a_lasting_id_that_undo_and_redo_keep`; `project::tests::tracks_get_lasting_ids_as_a_project_opens`; the whole-session test above also checks that an old song's tracks get 1, 2, 3 ... and keep them through a save and an open.

## Step 3: pad focus

The design's rule 2: **the pads follow the selected track until you choose for them.** Selecting a track puts its instrument on the pads, as on the MPC ("Follow track"); an instrument chosen for the pads stays there whichever track is selected ("Pads: Poly") until Follow track. Rule 1 says what never moves them: arming or selecting a track, a row of REC's list, REC.

### As built

- **`app::PadFocus`** (`app/pads_focus.rs`): `Track` (a new project's) or `Instrument(InstrumentId)`, in the app's state (`App::pads_focus`), apart from the track and the arm. Only the shared synth can be chosen yet (`PadFocus::SYNTH`, instrument 1): the Synth tab's button, while the selected track plays a kit (`App::toggle_pads_synth`). The picker of step 6 chooses others.
- **What moves it:** Follow track (the Synth tab's button again, or the header's chip, `App::pads_follow_track`); a kit picked for the selected track in the list of kits, or turned with SHIFT + the wheel (`App::kit_pick_follows_track`: a choice of what the pads play, as on the MPC, so they follow the track and play it); the arp coming on over a track that plays something else (`App::arp_ends_pads_synth`, below); a project opened (its own). **What doesn't:** `App::set_track` (selecting or arming), a row of REC's list (`desk.rs`, `ArmTrack`), REC (`App::rec_synth_on` keeps it).
- **How the pads play it** (`main.rs`, `preview_on`, mirrored by the test session): once a period, after the kit has followed the armed track, the loop reconciles. While the synth is chosen and the selected track plays something else (a kit, its own instrument), pad hits go to the shared synth on their own, before the arp and the recorder, as step 1's preview did; while the track plays the shared synth itself, they play it the usual way, through the track (the arp, the recorder, the synth's layout). The pads' colours follow, sent once. A pad held across a change lets go where it started.
- **REC** (step 1's rule 3): the desk's `rec_synth` acts only while the armed track plays something else. It points a track nobody set up at the shared synth (`Effect::SynthOnTrack`) and arms it; the pads keep the synth. The press comes again next period, when the armed track plays the pads' synth (`App::plays_pads_synth`, from `TrackKits::on_synth` each period), so it is REC's own, as is every later press there: a second take goes on the same track, not on another free one.
- **The arp:** it plays through the selected track, so while the synth plays on its own it can't play the pads: the button won't choose the synth while the arp plays them, and the arp coming on gives the pads back to the track, said. Over a track that plays the synth the arp plays it, and the pads keep it. The arp by instrument is for a later step (MIDI and scripts play instruments).
- **On screen:** the header's **Pads: Poly** chip on every page while the synth is chosen (warn, the shared synth's name from the list of kits; a tap is Follow track, `FromUi::FollowTrack`); Play's transport line reads "Pads: Poly · C3 to D#4" (`PadsPlay::chosen`) whichever track Play's rows show; the Synth tab's button reads **Follow track** over "The pads play Poly on every track". The Synth tab still shows the selected track's own instrument when it has one; which instrument the tab edits is the picker's (step 6).
- **Saved** in the `pads` part, written with every save:

  ```text
  mpfree-pads 1
  play instrument 1
  ```

  (`play track` while the pads follow the track). No part (New project, or a project saved before) follows the track, saying nothing; a part that doesn't read (another first line, a line it doesn't know, two `play` lines, none, or an instrument other than the shared synth) is said, and the pads follow the track (`App::open_pads`). An older build keeps it as a part it doesn't know.
- **Tests:** `app::tests::the_synth_chosen_for_the_pads_stays_until_follow_track`, `the_pads_focus_is_saved_and_a_part_that_doesnt_read_is_said`, `the_arp_and_the_synth_on_the_pads_never_play_apart`; `desk::tests::rec_while_the_pads_play_the_synth_records_it_on_a_track_nobody_set_up` (the second press records on the same track); `gui::tests::the_synth_chosen_for_the_pads_is_named_and_follow_track_gives_them_back` (the chip, the button, Play's transport, the chip's tap); `project::tests::what_the_pads_play_is_kept_and_comes_back`; `sampler::tests` (`TrackKits::on_synth`); and through the whole session, the design's proof, `smoke::workflows::tests::the_pads_play_poly_while_a_drum_clip_plays_on_whichever_track_is_selected` (a drum clip plays on Track 1 while the pads play Poly with Track 1, Track 3 and, from REC's list, Track 2 armed: every hit on the synth, none on a kit or a track, no track or clip changed; then a kit picked and the chip give the pads back), and `smoke::song_workflows::tests::the_synth_chosen_for_the_pads_comes_back_with_the_project` (saved, New project follows the track, opened again with Poly on the pads).
