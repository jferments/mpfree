# Make and Perform

**Status:** sequencer roadmap S4, first part (`rust/mpfree-app`, `rust/mpfree-seq`, 2026-11-17). Jay's sequencer decisions 4 and 5 (16 Nov 2026): Make and Perform as designed, with mutes landing on the bar in Perform. Design: the sequencer review (mpfree-claude `sequencer/rethink/README.md`, "Make and Perform: the toggle and when mutes land", mockups C and K). Not run on the MPC One yet.

## The rule

Make and Perform change how MPFree answers your hands, never what is in the project or what is playing. Make is for building: everything acts at once, and UNDO is the safety net. Perform is for a stage: changes land on the bar, one stray touch can't delete anything, and the screen shows what comes next.

## The model

- `mpfree_seq::Project::perform` (false for Make). The clips part saves it as a `perform` line before the tracks, and only in Perform, so a Make project's file is as before and an older build skips the line.
- `Project::launch` (sequencer S4's second part) saves as a `launch now|ticks N|bars N` line in the same place, only when it isn't one bar; a clip's own `launch` and `legato` lines go in its body. An older build skips them all.
- `TimeAction::Perform(bool)` sets it on the control half, which says what changed in place of the hint (`perform_line`). Switching changes nothing playing, waiting or recorded, and is not an undo step.
- `TimeState::perform` for the screens, `TimeHost::perform` for the app: the main loop and the smoke session pass it to `App::set_perform` each period, after `set_rolling`.

## What it switches, as built

| | Make | Perform |
| --- | --- | --- |
| The top bar's chip | MAKE | PERFORM, the second accent colour's dot (`Tone::Accent2`) |
| MAIN | Play; on Play, the next view | Arrange's Clips grid; on Arrange, the Song view and back (`Effect::ArrangeView`, `ToUi::ArrangeView`) |
| The arm | Moves as [rec-arm.md](rec-arm.md) says | Pinned: `App::set_perform` turns `RecArm::pinned` on when Perform starts and off when Make does, so the target list's Pin the arm still works by hand between switches |

The switch: a tap on the chip (`header/chip`, the leftmost; `perform_chip` in `gui.rs`) or **SHIFT + MAIN** (`App::mode_button`). Both send `TimeAction::Perform` with the other mode; the chip follows the next `TimeState`.

## Still to come in S4

In the order they are planned, each as its own part:

1. **Launch quantise and legato**, built (the second part, 17 Nov): the project's `LaunchGrid` (at once, 1/16 to 1/2, 1 to 8 bars) on the Record tab's Launch row and the transport bar's "Launch 1 bar" reading, and a clip's own Launch and Mode (Restart, Legato) on the clip settings card. The player lands each cue on its line, and a legato clip takes over mid-pass: see [playback.md](playback.md#launching-on-the-bar-line-cues-t9) and [scenes.md](scenes.md). It works the same in Make and Perform. Not built: the one-shot mode.
2. **The mute queue and solo**: in Perform a mute or solo waits for the launch line (SHIFT for at once; the other way round in Make), more taps join it, a tap on a waiting pad takes its change back, and they all land together on the line, frame-exact in the player. The pad blinks amber, the track's name in the grid is hatched with "mutes in 2 beats", and the transport bar says WAITING. Hold TRACK MUTE and tap a pad to solo. Rec song writes mute changes into the song.
3. **Perform's guard**: on-screen buttons that delete or clear need a half-second hold in Perform; Rec song goes on with REC; the Clips grid shows five big rows with rings and countdowns, and the transport bar what comes next.
4. Reload clip, live transpose, the song list (MAIN's third view in Perform), the scene editor and Back to song.

## Tests

The second part's are listed on [playback.md](playback.md) and [scenes.md](scenes.md), with `mpfree_seq::tests::launch_grids_find_their_next_line`, the time-ui tests `record_settings_step_within_their_range`, `the_clip_card_steps_the_focused_clips_settings` and `cued_cells_show_where_they_start_and_taps_take_cues_back`, and the app's `a_launch_lands_on_the_launch_quantise_and_waiting_cues_keep_their_line`. The first part's:

- `mpfree_seq::save::tests::an_empty_project_and_a_new_one_round_trip`: Perform reads back, Make writes no line, and a `perform` line inside a track is skipped.
- `app::tests::shift_main_switches_make_and_perform_and_perform_moves_main_home`: SHIFT + MAIN asks for the other mode; `set_perform` pins and unpins the arm only when the mode changes; MAIN goes to Arrange's Clips grid and then steps its views in Perform, and to Play in Make.
- `smoke::workflows::tests::make_and_perform_switch_from_the_chip_and_shift_main`: the MAKE chip switches to Perform while the transport plays, the chip says PERFORM and the hint says what changed, MAIN steps the Clips grid and the Song view, the Looper tab doesn't move the pinned arm, and SHIFT + MAIN comes back to Make with nothing stopped.
