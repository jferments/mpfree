# REC records what is armed

**Status:** sequencer roadmap S2, first part (`rust/mpfree-app`, 2026-11-17). Jay's sequencer decision 1 (16 Nov 2026, "REC records what is armed"): REC, OVERDUB and PLAY act on one armed target from every page, and the transport bar always names it. Design: the sequencer review (mpfree-claude `sequencer/rethink/README.md`, "REC, OVERDUB and PLAY", mockups F to H). Not run on the MPC One yet: the `rec-arm` device check goes into the next combined round, and the `transport` check now opens the Loop view first.

Until now the page decided what REC recorded: the looper on Play's All and Loop views and on the Looper tab, a clip on Beats and Arrange. The home view showed the loop strip and the pattern together and REC went to the loop, so in the 16 Nov round the pattern stayed silent; nothing on screen said which.

## The rule

REC records the last recordable thing you opened or armed, and the transport bar names it.

- **The notes:** the armed track (the one the pads play), into its clip: the one the pattern shows, or a new clip in its first empty slot, as the time engine's Record does (`ready_to_record`).
- **A loop:** the looper's picked loop.

One thing is armed at a time. A new or opened project arms the notes. The arm isn't saved and isn't an undo step. SHIFT + REC is still Capture, whatever is armed.

## What moves the arm

1. Opening Play's **Beats** view arms the notes; Play's **Loop** view and the **Looper** tab arm the loop (`Player::arm_for_page`, once each time the page is shown, so an arm moved on it since stays).
2. **Picking a track** (`TimeAction::Arm(Some(_))`) arms its notes; **the loop's own Record or Overdub** (the Loop view's strip, the Looper tab) arms the loop, letting go of the notes' ready.
3. Nothing else: Play's All view, Mix, Synth, Pads, Sample, the Song view, the menus and Prefs leave it where it is.
4. **Nothing moves it while a take records** (the notes recording or counting in, or a loop recording its first pass or overdubbing).
5. **A ready goes with the arm:** the notes ready moving to the loop arm the loop (Record, or Overdub for an added take); an armed loop moving to the notes lets go of the loop and leaves the notes ready.

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

## How it shows

- **The REC chip** on Play's transport card, between the tempo and the bar dots (the dots narrowed to 8 px a 16th for it): "REC → Drums · Clip 1" in grey with a red dot while armed; "READY · PLAY starts → Loop 1" outlined in red while ready ("READY TO ADD" for OVERDUB); solid red "● REC" or "● OVERDUB" while recording. Arrange's transport bar shows the same as a chip at its right end ("REC → Loop 1"), and its Rec, Play and Stop are REC, PLAY and STOP. A tap on either chip says what REC records and what arms another target.
- **The loop strip's empty text** says what REC does: "Empty: REC records the notes now. Open the Loop view to record a loop here" while the notes are armed, "Empty: REC records the pads and the synth" while the loop is (`LooperPage::set_rec_loop`).
- **The hint line** while the notes are ready: "PLAY: record Drums · Clip 1 · PLAY START: from bar 1 · STOP: not ready".
- **The lights:** the notes ready blink REC (OVERDUB for an added take) with PLAY, as an armed loop does.

## Where it lives

- `rec_arm.rs`: `Target`, `Take`, `RecArm` (`press`, `move_to`, `ready_lights`), `Now` (what a press finds) and `Do` (what it does, in order).
- `app.rs`: REC and OVERDUB (without SHIFT) on every page give `Effect::Rec`; PLAY, PLAY START and STOP do too while the notes are ready; `transport_lights` blinks for ready.
- `desk.rs`: Arrange's bar Rec (`TimeAction::Transport(Record)`) is REC, and its Play and Stop act as PLAY and STOP while the notes are ready. `Effect::Rec` and `FromUi::Arm` find the transport and the loops (`rec_now`, `LoopHost::rec_now`), run the arm and carry out what it gives as the app's time and looper effects; the screen hears `ToUi::Rec`.
- `gui.rs`: `arm_for_page`, `rec_chip` and `refresh_rec_chip`, `rec_ready_hint`; `play_widgets::RecChip` paints it.

## Later in S2

The Arm pills on the loop strip and the pattern heading, the chip's target list (mockup H), SHIFT + Arm for a second target, Pin, the chip in the header on pages without a transport bar, a blinking chip, UNDO throwing the take away while recording, the sampler's recorder as a target (the sampler thread to confirm how it readies), and audio tracks.

## Tests

- `rec_arm::tests`: ready from stopped and PLAY or PLAY START starts it; REC again or STOP lets go and OVERDUB readies an added take, the transport's mode set as it starts; punch in and out while playing and recording; the loop records as the looper does; the arm moves with its ready, but not while a take records.
- `app::tests`: REC and OVERDUB give `Effect::Rec` on Play, the Looper tab and Arrange; PLAY, PLAY START and STOP while ready; the lights.
- `gui::tests::undo_follows_the_page_and_the_play_view`: the pages that arm what they show.
- `smoke::workflows::tests::rec_records_what_is_armed_and_play_starts_a_ready_take`: the home view arms the notes; REC readies, STOP lets go, REC then PLAY records a pad into the clip; the Loop view opened while it records leaves the arm; opened again, it arms the loop and REC arms it; Beats takes the arm back with its ready and the loop lets go. The chip follows each step.
- `smoke::workflows::tests::the_loop_strip_says_what_rec_records_and_its_record_arms_the_loop`: the strip's empty text follows the arm; a notes take keeps the arm when the Loop view opens, and the strip's Record then arms the loop.
- `smoke::workflows::tests::arranges_bar_rec_and_play_record_what_is_armed`: Arrange's Rec readies the notes, its Stop lets go and its Play records; its chip names the loop once the Looper tab arms it, and Rec arms the loop.
- `smoke::phases::device_log::rec_arm_writes_its_lines`: the lines the `rec-arm` device check looks for ([device checks](../hardware/device-checks.md)), and the take heard back on PLAY.
- The screen fingerprints: the Play screens with the chip, `play-rec-ready`, and Arrange's bar.
