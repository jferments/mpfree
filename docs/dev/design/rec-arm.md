# REC records what is armed

**Status:** sequencer roadmap S2, first to third parts (`rust/mpfree-app`, 2026-11-17). Jay's sequencer decision 1 (16 Nov 2026, "REC records what is armed"): REC, OVERDUB and PLAY act on one armed target from every page, and the transport bar always names it. Design: the sequencer review (mpfree-claude `sequencer/rethink/README.md`, "REC, OVERDUB and PLAY", mockups F to H). Not run on the MPC One yet: the `rec-arm` device check goes into the next combined round, and the `transport` check now opens the Loop view first.

Until now the page decided what REC recorded: the looper on Play's All and Loop views and on the Looper tab, a clip on Beats and Arrange. The home view showed the loop strip and the pattern together and REC went to the loop, so in the 16 Nov round the pattern stayed silent; nothing on screen said which.

## The rule

REC records the last recordable thing you opened or armed, and the transport bar names it.

- **The notes:** the armed track (the one the pads play), into its clip: the one the pattern shows, or a new clip in its first empty slot, as the time engine's Record does (`ready_to_record`).
- **A loop:** the looper's picked loop.

One thing is armed at a time, unless **SHIFT + Arm** adds the other (`RecArm::both`, `RecArm::add`): then REC records the notes into the clip and the loop in one pass, as the MPC arms several tracks with SHIFT and Live with Ctrl. A new or opened project arms the notes alone. The arm isn't saved and isn't an undo step. SHIFT + REC is still Capture, whatever is armed.

## What moves the arm

1. **A tap on an Arm pill** (the loop strip's, the pattern's) or a row of the chip's target list arms what it names, even when pinned (`Mover::Tap`, `FromUi::ArmTap`); so does **the loop's own Record or Overdub** (the Loop view's strip, the Looper tab), letting go of the notes' ready.
2. Opening Play's **Beats** view arms the notes; Play's **Loop** view and the **Looper** tab arm the loop (`Mover::Page`, `Player::arm_for_page`, once each time the page is shown, so an arm moved on it since stays).
3. **Picking a track** (`Mover::Track`, `TimeAction::Arm(Some(_))`) arms its notes.
4. Nothing else: Play's All view, Mix, Synth, Pads, Sample, the Song view, the menus and Prefs leave it where it is.
5. **Pinned** (the target list's Pin the arm, `RecArm::pinned`, `FromUi::RecPin`), only a tap moves it: pages and track picks leave it. Pin isn't saved either.
6. **Nothing moves it while a take records** (the notes recording or counting in, or a loop recording its first pass or overdubbing); a tap then says "Finish the take first".
7. **SHIFT + a tap** on a pill or a row adds that target to the armed one (`FromUi::ArmAdd`), sharing a ready: the loop added to the notes ready waits for PLAY too, the notes added to an armed loop get ready. With both armed, SHIFT + a tap lets go of the one tapped and a plain tap keeps only the one tapped (the other lets go of its ready or its wait); a page or a track picked finds its target armed already and changes nothing. A SHIFT + tap on the only target says how to add one. Refused while a take records, like a tap.
8. **A ready goes with the arm:** the notes ready moving to the loop arm the loop (Record, or Overdub for an added take); an armed loop moving to the notes lets go of the loop and leaves the notes ready.

## The presses

With the notes armed (`RecArm::press`):

| Press | Stopped | Playing | Recording or counting in |
| --- | --- | --- | --- |
| REC | Ready (again: not ready); PLAY then records, after the count-in | Punch in, replacing | Punch out; the music plays on |
| OVERDUB | Ready to add (again: not ready) | Punch in, adding | Switch between replacing and adding |
| PLAY / PLAY START while ready | Records (PLAY START from bar 1) | | |
| STOP while ready | Not ready (and STOP's own job) | | |

Ready is the app's own (`App::rec`); the transport only hears Record and ToggleOverdub when the take starts, so REC held and PLAY pressed records at once, the MPC's way. The transport's Replace / Overdub mode is set to the take's as it starts.

With a loop armed, REC is the looper's Record and OVERDUB its Overdub, as the Looper tab's were: REC stopped arms the loop and PLAY starts its first pass; the looper's armed loop is its ready.

With both armed (`RecArm::press_both`), each does what it does alone, in step. Stopped, REC readies the notes and arms the loop (REC again or STOP lets go of both); PLAY starts the notes' take and, with it, the loop's first pass, after the count-in (the looper waits for the transport to start, not for the count-in). Playing with nothing recording, REC punches the notes in and the loop starts on the next bar line (or overdubs, if it has a take). While either records, REC goes only to what records: the notes punch out and the loop's first pass closes on the bar line (an overdub punches out). OVERDUB does the same with the notes' added take and the looper's Overdub.

## UNDO while recording

While a take records, UNDO throws it away (`RecArm::throw`, from the desk's `Effect::Undo`): the notes punch out (`TimeAction::ThrowTake`) and, once the punch out lands and the take's last changes have come (`TimeControl::period`, `throw_pass`), their take is undone and forgotten, so REDO can't bring it back; the log says "undo: take 3 thrown away". During the count-in it stops. A loop recording its first pass or overdubbing gets the looper's Undo. Otherwise UNDO is the page's, as before.

## How it shows

- **The REC chip** on Play's transport card, between the tempo and the bar dots (the dots narrowed to 8 px a 16th for it): "REC → Drums · Clip 1" in grey with a red dot while armed ("REC · PINNED" while pinned; "Drums + Loop 1" with both armed, as far along as the one further on); "READY · PLAY starts → Loop 1" outlined in red while ready ("READY TO ADD" for OVERDUB); solid red "● REC" or "● OVERDUB" while recording. Arrange's transport bar shows the same as a chip at its right end ("REC → Loop 1"), and its Rec, Play and Stop are REC, PLAY and STOP. A tap on either chip says what REC records and what arms another target.
- **The loop strip's empty text** says what REC does: "Empty: REC records the notes now. Open the Loop view to record a loop here" while the notes are armed, "Empty: REC records the pads and the synth" while the loop is (`LooperPage::set_rec_loop`).
- **The Arm pills** at the left of the loop strip's and the pattern's heading (`play_widgets::ArmPill`, `paint_arm`): grey "Arm" on what REC doesn't record, red "Armed", "Ready" or "Rec" on what it does (both red with both armed, each with its own state); the armed panel gets a red edge, a dashed red outline while ready and a solid one while recording (mockups F and G).
- **The target list** (mockup H, `rec_list.rs`, page `Play · rec`): a tap on Play's chip opens it in place of the view; rows for the notes and the loop, Rec song, Pin the arm, Close, and what moves the arm and the chip's three looks. The chip again, Close or a view picked closes it.
- **The header chip** while REC is ready or recording, on pages without a transport bar: "READY → Track 1", "● REC → Loop 1"; a tap says what REC records.
- **The hint line** while the notes are ready: "PLAY: record Drums · Clip 1 · PLAY START: from bar 1 · STOP: not ready".
- **The lights:** the notes ready blink REC (OVERDUB for an added take) with PLAY, as an armed loop does.

## Where it lives

- `rec_arm.rs`: `Target`, `Take`, `Mover`, `RecArm` (`press`, `move_to`, `add`, `armed`, `throw`, `ready_lights`), `Now` (what a press finds) and `Do` (what it does, in order).
- `rec_list.rs`: the chip's target list (`RecList`, `ListRow`).
- `time/control.rs`: `TimeAction::ThrowTake`, `throw_pass`.
- `app.rs`: REC and OVERDUB (without SHIFT) on every page give `Effect::Rec`; PLAY, PLAY START and STOP do too while the notes are ready; `transport_lights` blinks for ready.
- `desk.rs`: Arrange's bar Rec (`TimeAction::Transport(Record)`) is REC, and its Play and Stop act as PLAY and STOP while the notes are ready. `Effect::Rec` and `FromUi::Arm` find the transport and the loops (`rec_now`, `LoopHost::rec_now`), run the arm and carry out what it gives as the app's time and looper effects; the screen hears `ToUi::Rec`.
- `gui.rs`: `arm_for_page`, `arm_tap` (SHIFT held: `FromUi::ArmAdd`), `arm_pill`, `rec_chip`, `rec_chip_of` and `refresh_rec_chip` (the pills too), `toggle_rec_list` and `refresh_rec_list`, `rec_ready_hint`, the header chip in `refresh_chips`; `play_widgets::RecChip` and `paint_arm` paint them.

## Later in S2

Pin on in Perform (S4), a blinking chip, the pass number on the chip while recording ("● REC · pass 2"), a target that can't record (a full looper, no free clip slot) dimmed in the list with the reason, the sampler's recorder as a target (the sampler thread to confirm how it readies), and audio tracks. With more than two targets, `RecArm::both` becomes a set.

## Tests

- `rec_arm::tests`: ready from stopped and PLAY or PLAY START starts it; REC again or STOP lets go and OVERDUB readies an added take, the transport's mode set as it starts; punch in and out while playing and recording; the loop records as the looper does; the arm moves with its ready, but not while a take records.
- `app::tests`: REC and OVERDUB give `Effect::Rec` on Play, the Looper tab and Arrange; PLAY, PLAY START and STOP while ready; the lights.
- `gui::tests::undo_follows_the_page_and_the_play_view`: the pages that arm what they show.
- `smoke::workflows::tests::rec_records_what_is_armed_and_play_starts_a_ready_take`: the home view arms the notes; REC readies, STOP lets go, REC then PLAY records a pad into the clip; the Loop view opened while it records leaves the arm; opened again, it arms the loop and REC arms it; Beats takes the arm back with its ready and the loop lets go. The chip follows each step.
- `smoke::workflows::tests::the_loop_strip_says_what_rec_records_and_its_record_arms_the_loop`: the strip's empty text follows the arm; a notes take keeps the arm when the Loop view opens, and the strip's Record then arms the loop.
- `smoke::workflows::tests::arranges_bar_rec_and_play_record_what_is_armed`: Arrange's Rec readies the notes, its Stop lets go and its Play records; its chip names the loop once the Looper tab arms it, and Rec arms the loop.
- `rec_arm::tests::pinned_only_a_tap_moves_the_arm_and_a_tap_while_recording_says_why`, `undo_while_a_take_records_throws_it_away`; `rec_list::tests`.
- `smoke::workflows::tests::arm_pills_and_the_target_list_move_the_arm_and_pin_it`: the pills, the header chip, the list's rows, Pin, Rec song and Close.
- `smoke::workflows::tests::undo_while_recording_throws_the_take_away_and_the_music_plays_on`.
- `rec_arm::tests::shift_arm_adds_the_other_target_and_rec_records_both_in_step`, `a_plain_tap_keeps_one_of_two_and_shift_lets_go_of_one`.
- `smoke::workflows::tests::shift_arm_records_the_notes_and_a_loop_in_one_pass`: SHIFT + the strip's Arm arms both, Beats leaves them, REC readies the notes and arms the loop, PLAY after a bar's count-in starts both (the loop not during the count-in), a tap while they record is refused, REC closes both with the pad in the clip and the loop playing, and a plain tap keeps only the notes.
- `smoke::phases::device_log::rec_arm_writes_its_lines`: the lines the `rec-arm` device check looks for ([device checks](../hardware/device-checks.md)), and the take heard back on PLAY.
- The screen fingerprints: the Play screens with the chip, `play-rec-ready`, `play-rec-both`, `play-rec-list` (pinned), and Arrange's bar.
