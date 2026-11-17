# Make and Perform

**Status:** sequencer roadmap S4, parts one to four (`rust/mpfree-app`, `rust/mpfree-seq`, `rust/mpfree-play`, `rust/mpfree-perform`, `rust/mpfree-time-ui`, `rust/mpfree-ui`, 2026-11-17). Jay's sequencer decisions 4 and 5 (16 Nov 2026): Make and Perform as designed, with mutes landing on the bar in Perform. Design: the sequencer review (mpfree-claude `sequencer/rethink/README.md`, "Make and Perform: the toggle and when mutes land", mockups C and K). Not run on the MPC One yet.

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
| TRACK MUTE's pads | At once; SHIFT waits for the launch line | Wait for the launch line; SHIFT acts at once (below) |
| Rec song | Off until turned on | On with REC's take (below) |
| Delete and clear buttons | One tap; UNDO brings it back | A half-second hold (Perform's guard, below) |
| The Clips grid | 8 compact rows (`ROW_MIN_H_MAKE`): names and lengths, a small ring | 5 big rows (`ROW_MIN_H`), and a queued cell's countdown (below) |
| The transport bar | As before | NEXT after WAITING (below) |

The switch: a tap on the chip (`header/chip`, the leftmost; `perform_chip` in `gui.rs`) or **SHIFT + MAIN** (`App::mode_button`). Both send `TimeAction::Perform` with the other mode; the chip follows the next `TimeState`, and the hint line says what changed (`perform_line`: "Perform: mutes land on the bar, deletes need a hold, MAIN goes to the Clips grid").

## When mutes land, as built (the third part)

TRACK MUTE's pads (layout spec section 6) mute or solo a track at once or on the launch line:

- **The timing.** `App::mute_tap` gives each tap a `MuteTap`: `wait` is Perform with SHIFT's other timing (`perform != shift`), and `solo` is TRACK MUTE held. `main.rs`'s `mute_key` hands it to `TimeHost::mute_or_solo` (`TimeControl::mute_or_solo`). A tap acts at once when it doesn't wait, when the transport is stopped, or when Launch is at once.
- **The queue.** A tap that waits joins `TimeControl::mute_queue`: an id, and a `MuteWait` with the line (the project's Launch when the queue starts, kept if Launch changes) and each track's wanted mute and solo. A tap that brings a track back to what it has now takes its change back, and an empty queue goes. Each view compiled carries a `MuteCue { silent, line, id }` for each track the queue changes (`PlayView::set_mute_cue`), with its silence worked out after the queue, solo included.
- **Frame-exact landing.** The player (`Player::play_span`) splits a track's span at its mute cue's line (`mute_due`), plays the part before, lands the mute (`land_mute`: a track going silent has its sounding notes end on the line, its hits, touch and locks let go) and plays the rest. It reports `MuteLanded { track, silent, at, id }` (`ToControl::MuteLanded`, sent by `TimeAudio` after the started cues). Its `muted` holds a landed cue while the view has the same id; where a scene's cue mute also switches the track, the later tick wins.
- **The control half** applies the whole queue on the first landing with its id (`land_queue_now`) and compiles. Rec song takes each landing as a silence (`SongRecorder::mute`, a gap in the placements), and a Loop track's loops go on or off on the landing tick (`loop_at`, as a cue's do). **STOP lands whatever waits**: a queue with the transport stopped lands in the next period.
- **Solo.** `Track::solo`, saved as a `solo` line after `mute` when on. `Project::silent(t)` is the one rule (muted, or another track soloed and not this one), used by the compile, the Loop tracks (`loops::playing`), song mode's free tracks and Rec song. A scene's cue mutes are made solo-aware in `TimeControl::view` (a track not soloed stays silent while any track is).
- **The screens.** `TimeState::mute_wait` carries the queue; `mute_waiting(t)` gives the grid header's line ("mutes in 2 beats", rounded up to beats under a bar and bars past one), and `waiting_reading` the transport bar's WAITING item ("Hats, Vox at 18.1"), which goes first in `bar_of`. `GridTrack` has `solo` (outlined in the accent) and `waiting` (the header hatched, `widgets::hatch`, and outlined in amber, its line shortened to fit by `waiting_fit`); `mute` is the track's silence. Sample screens: `clips-mutes` (Perform, two changes waiting) and `clips-solo`.
- **The pad lights** (`mute_colours`, from `TimeHost::mute_light`'s `MuteLight`): amber blinking twice a second while waiting, the hit of blue while soloed, red while silent (muted or by solo), green lit while a clip plays, green with nothing launched. Akai's app was never seen sending a dim red, so silent by solo is the muted red.
- **Hold for solo** (`App::mute_hold`): TRACK MUTE pressed without SHIFT toggles the mode as before and remembers the mode it had. A pad tapped while it is held solos (turning the mode to Tracks for the lights if the press had turned it off), and the release puts the mode back (`track_mute_up`). `App::pads_mute` routes pads to the mute keys while the mode is on or TRACK MUTE is held.
- **Not undo steps.** A mute or solo changes no take, as before.
- **Not built yet:** the kit's pads (SHIFT + TRACK MUTE) mute at once and have no solo, since the mixer's `PadMute` has no line to wait for. During count-in a change lands with the queue's line as the player gives it (the first bar line after it), not checked on the MPC. A tap taking a change back in the period between its landing and the landing's report acts at once.

## Rec song with REC, as built (the fourth part)

The design's "Rec song is on with REC in Perform":

- **The take.** The host says each period whether a take records (`rec_arm::Now::taking`: the notes, their count-in, or a loop's first pass or overdub; `desk::rec_now`), through `TimeHost::take_follows`, called by the main loop and the smoke session after `App::set_perform`. In Perform, a take starting with Rec song off turns it on (`SongTake::Armed`, "rec song: on with REC (Perform)"); the take ending turns it off and keeps what it recorded in the song ("rec song: off with the take, 3 placements ..."). Rec song on by hand isn't touched.
- **"+ Rec song".** `TimeState::song_with_take` says the take recording, or the next, brings it; the REC chip's `what` then ends "+ Rec song" (`Gui::rec_chip`, in Perform with Rec song on as well). The target list's switch reads **Rec song with REC** in Perform (`RecList::perform`), and `gui::rec_song_switch` picks what a tap does: `TimeAction::RecSongWithTake(false)` takes it off for the take recording or the next (`SongTake::Off`; a take's Rec song stops there and keeps what it had), `RecSongWithTake(true)` puts it back (arming it at once during a take), and Rec song on by hand goes off as before. `RecSong(false)` during a take that turned it on is off for that take too. After the take, `Off` goes back to `Idle`.
- **Make** is as before: a take changes nothing, and `RecSongWithTake` is refused.

## Perform's guard and big state, as built (the fourth part)

- **The guard.** In Perform a finger down on a button that deletes or clears is kept from it (`Gui::watch_guard`, before MIDI Learn and the widgets): `guarded(widget, target)` lists the Song view's Delete and an audio region's Erase, the note editor's Delete, the Looper tab's Clear loop, a layer's × and ½ length, Play's loop strip's Clear and the step inspector's Clear. The hint line says "Hold to delete" (`GUARD_HINT`), and `Ui::set_hold_fill` draws a red fill (the theme's `bad`, part see-through) crossing the button from the left, drawing that widget again whole as it moves. `Gui::guard_tick`, from `tick`, fills it over `GUARD_HOLD` (0.5 s), then presses the button: the touch's Down and Up go to the widget where the finger landed, as a tap would, and the rest of that finger's touch is the guard's (`guard_done`). Lifted early, or slid more than 20 px off the button, nothing happens and the hint says to hold. The wheel's push and ERASE + a tap are not guarded. The design's Delete clip has no on-screen button (ERASE + a clip), and its Halve is the clip card's SHIFT + ‹, two hands; the song list's Remove comes with the song list.
- **The grid's rows.** `ClipGrid::big` (set from `TimeState::perform` by `TimePage::update`, before it counts the rows in view, keeping the first row within the slots): Perform's rows are at least `ROW_MIN_H` (76 px, 5 on the MPC's screen), Make's `ROW_MIN_H_MAKE` (44 px, 8). A cell under `COMPACT_H` (64 px) is drawn compact (`paint_compact`): its name, then its length, or its foot while queued ("from 7.1", amber), or a loop's wave; a playing clip's ring is `SMALL_RING` (18 px) at its right.
- **The countdown.** In Perform, while the transport rolls, a queued cell's `GridCell::progress` is how near its line is over its launch grid's span (`TimeState::queued_foot`; a launch with no grid, a bar), at least 1/64 so it shows from the tap; `paint_cell` draws it as an amber bar along the cell's foot. The foot still says where it starts ("from 6.1", as mockup C).
- **NEXT.** `TimeState::next_reading` (Perform, rolling): a scene cued (its name, at its cues' line or the project's Launch), else the cues (the first clip's name or "Bass stops", "+N" for more, at the soonest line), else `TimeState::follow_next` (the scene playing's follow action: `Perform::scene_follow` gives its row, point and follow while no scene is cued; `follow_words` names the row a Next, Previous, First or Last plays with `follow_target`, "row 1 again", "Stop all", "Any scene", "Another scene", "Drop or stay" with a chance between, nothing for Stay), else in song mode the next marker. Each says how soon with `countdown` (beats rounded up under a bar, then bars; the mute header's words too). `bar_of` puts it after WAITING with `Tone::Accent`, and the transport bar cuts its first reading to fit rather than leaving it out.

## Still to come in S4

In the order they are planned, each as its own part:

1. **Launch quantise and legato**, built (the second part, 17 Nov): the project's `LaunchGrid` (at once, 1/16 to 1/2, 1 to 8 bars) on the Record tab's Launch row and the transport bar's "Launch 1 bar" reading, and a clip's own Launch and Mode (Restart, Legato) on the clip settings card. The player lands each cue on its line, and a legato clip takes over mid-pass: see [playback.md](playback.md#launching-on-the-bar-line-cues-t9) and [scenes.md](scenes.md). It works the same in Make and Perform. Not built: the one-shot mode.
2. **The mute queue and solo**, built (the third part, 17 Nov): see above.
3. **Perform's guard and big state**, built (the fourth part, 17 Nov): the hold on deletes, Make's and Perform's rows with the countdown, NEXT, and Rec song with REC (above).
4. Reload clip, live transpose, the song list (MAIN's third view in Perform), the scene editor and Back to song.

## Tests

The fourth part's: `gui::tests::perform_guards_deletes_with_a_half_second_hold` (a tap in Perform does nothing and says to hold, a hold deletes, sliding off cancels, Make is one tap), the time-ui tests `perform_shows_big_rows_countdowns_and_what_comes_next` (8 rows and 5, the countdown, each NEXT and WAITING first) and `countdowns_round_up_to_beats_then_bars`, the widget test `grid_cells_and_names_report_where_they_are` (both row heights), `mpfree_ui`'s `a_hold_fill_draws_over_its_widget_and_goes`, the perform crate's `a_scene_follow_fires_in_the_bar_before_its_point` (`scene_follow`), `time::tests::in_perform_a_take_brings_rec_song_for_its_length` (a take arms and keeps, "+ Rec song" off skips one take, off by hand during a take, put back during a take, by hand stays, Make refuses) and the smoke test `in_perform_rec_brings_rec_song_with_the_take` (the chip's "+ Rec song", REC and PLAY, the placements kept on STOP, the list's switch off and back).

The third part's:

- `mpfree-play/tests/mute_cue.rs` (6 tests): a mute lands on the bar line and ends the notes there, a note held over the line ends on it, a queue lands together on its line, a mute at once lands on the next tick, in a loop a mute lands on the loop start, and a scene's mute and a mute cue take the later.
- `time::tests::mutes_that_wait_land_together_on_the_launch_line`: a waiting mute lands on bar 2 (its notes stop there), a take-back, a solo from bar 3 silences the others, STOP lands a solo off that waits, and stopped a tap acts at once.
- `time::tests::a_loop_tracks_mute_that_waits_turns_its_loop_off_on_the_line`: the loop goes off on the line, given in the one period that has it.
- `app::tests::track_mute_held_with_a_pad_solos_and_leaves_the_mode_as_it_was`: hold for solo from either mode, Make's and Perform's timing with SHIFT, and SHIFT + TRACK MUTE held is no solo.
- `tests::the_mute_keys_mute_tracks_or_pads_and_light_what_plays` (`main.rs`): the log lines, solo silencing the others, and every pad light.
- The time-ui test `mutes_waiting_show_on_the_grid_and_the_transport_bar`, and `mpfree_seq::save` round-trips `solo`.
- `smoke::phases::device_log::mute_queue_writes_its_lines`: the lines the device check `mute-queue` reads ([device-checks.md](../hardware/device-checks.md)): the tap that waits, its landing ("time: track mute: at 2.1, Track 1 muted", written by `TimeControl::landed_line` through `take_later_note`) and the solo.
- `smoke::workflows::tests::in_perform_a_track_mute_waits_for_the_bar_and_held_it_solos`: in Perform a TRACK MUTE pad waits for bar 2 with WAITING on the transport bar, the kick goes silent there, held with SHIFT a pad solos at once and the mode stays, and STOP lands what waits.

The second part's are listed on [playback.md](playback.md) and [scenes.md](scenes.md), with `mpfree_seq::tests::launch_grids_find_their_next_line`, the time-ui tests `record_settings_step_within_their_range`, `the_clip_card_steps_the_focused_clips_settings` and `cued_cells_show_where_they_start_and_taps_take_cues_back`, and the app's `a_launch_lands_on_the_launch_quantise_and_waiting_cues_keep_their_line`. The first part's:

- `mpfree_seq::save::tests::an_empty_project_and_a_new_one_round_trip`: Perform reads back, Make writes no line, and a `perform` line inside a track is skipped.
- `app::tests::shift_main_switches_make_and_perform_and_perform_moves_main_home`: SHIFT + MAIN asks for the other mode; `set_perform` pins and unpins the arm only when the mode changes; MAIN goes to Arrange's Clips grid and then steps its views in Perform, and to Play in Make.
- `smoke::workflows::tests::make_and_perform_switch_from_the_chip_and_shift_main`: the MAKE chip switches to Perform while the transport plays, the chip says PERFORM and the hint says what changed, MAIN steps the Clips grid and the Song view, the Looper tab doesn't move the pinned arm, and SHIFT + MAIN comes back to Make with nothing stopped.
