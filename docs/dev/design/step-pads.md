<!--
covers: rust/mpfree-app/src/app/step_pads.rs rust/mpfree-app/src/step_keys.rs rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/play_widgets.rs rust/mpfree-seq/src/steps.rs
checked: 2026-11-16 d7c844c
-->

# STEP SEQ: the pads as steps

**Status:** S1 of the sequencer roadmap, first part, 2026-11-16. Decided by the owner on 16 Nov 2026 (sequencer decision 2, card "Pads become steps, as designed", 20:00Z). The design notes, with mockup I (`step-pads.png`) and the Beats mockup, are in the private notes repository (`sequencer/rethink/README.md`, "STEP SEQ: the pads as steps"). The buttons and the screen are in the [layout spec](layout-spec.md) (sections 2, 4 and 6). Tested on the host; not yet run on the MPC (device check `step-pads`).

This builds on the [step sequencer](steps.md): steps are a view of a clip's notes, and each edit is one take.

## The rule

**The lit mode button says what the pads are.** With STEP SEQ lit, the 16 pads are 16 steps of one sound of the clip Beats shows (the armed track's playing clip, else its first), on every page, until STEP SEQ is pressed again. TRACK MUTE works the same way, and there is one pad mode at a time: STEP SEQ, TRACK MUTE and 16 LEVEL each turn the others off.

| Press | Does |
| --- | --- |
| STEP SEQ (on its release) | The pads become steps, and Play shows Beats; again, they play |
| Hold STEP SEQ, tap a pad | The pads are the kit while it is held (PAD BANK picks the kit's bank); the pad tapped sounds and is the sound when STEP SEQ is let go |
| SHIFT + STEP SEQ | Follow: the page turns with the playhead |
| Tap an empty step | A note at the pad's velocity (127 with FULL LEVEL), heard once while the transport is stopped |
| Tap a lit step | It comes off, with its locks, when the pad is let go |
| PAD BANK A–D | Pages 1–4; SHIFT or a second press 5–8. Turns Follow off |
| ERASE + a step pad | Clears the step, locks included |
| ERASE + PAD BANK | Clears that page of the sound's steps (SHIFT: the other four pages) |
| UNDO | Takes back the last step edit |

Pad 1, bottom left, is the page's first step. With no clip, the first tap on page 1 makes a one-bar clip in the track's first empty slot and launches it, as a tap on Beats does; a tap past the clip's end says how long the clip is.

## Where it lives

- **The app** (`app/step_pads.rs`) keeps the mode, the sound, the page and Follow, and answers the buttons: STEP SEQ's press and release (`App::button` calls `step_seq_up` on the release), PAD BANK while the pads are steps, ERASE + PAD BANK as `Effect::Steps(StepAction::ClearSteps)`. The pad played last comes from `App::pad_down`. `App::step_pads` is what the screen and the audio loop read (`Status::steps`); `App::pads_are_steps` is true while a press is a step edit (on, and STEP SEQ not held).
- **The audio loop** (`main.rs`, with the press logic in `step_keys.rs`) sends a pad press to `StepKeys::press` before TRACK MUTE's keys and before the pads play: it reads the steps (`TimeHost::steps`), and puts a note on with `StepAction::Tap`, or holds a lit step for its release. The smoke session (`smoke/session.rs`) routes pads the same way.
- **The lights** (`step_keys::colours`, `step_keys::bank_lights`) are worked out each period from the steps kept since the last change (`take_steps_changed`) and the transport's position, and only the pads that changed are sent, as for TRACK MUTE.
- **The screen** (`gui.rs`, `play_page.rs`, `play_widgets.rs`) shows the "Pads: steps · Kick" chip (a tap: `FromUi::StepsOff`), the hint line, and on Beats the lane on the pads, first and outlined, and the pads' page. ‹ and › on the pattern's head send `FromUi::StepPage`, so the pads follow the screen.
- **The engine** adds `Grid::clear_steps` (one sound's notes and locks on a range of steps) and `StepClip::locks` (each step's lock count, for the lock colour).

## The lights

| Step | Colour |
| --- | --- |
| A note | The sound's hit colour, a quarter bright at velocity 1, full at 127 |
| A note with locks | Purple, by velocity |
| Locks with no note | Dim purple |
| The rest of a note held over | The sound's colour, dim |
| The playhead | White |
| An empty first step of a beat (1, 5, 9, 13) | A faint glow |
| Past the clip's end | Dark |

The colours go out on the pad colour message Akai's app uses (`akai::pad_colour`), with values in its 0–127 range; white, purple and the glow are colours Akai's app was not seen sending, so the device check looks at them. The PAD BANK lights keep to the two values Akai's app was seen sending for those buttons, 1 and 3: the page shown is lit, the others dim (pages past the clip's end too), and the page playing blinks twice a second when it is another of the four.

## Coming next in S1

- Hold a step and turn a Q-Link, or anything on another page, to set or lock that step; touching a Q-Link shows its value; several held steps set together.
- Hold a step and tap a later one for a tie; SHIFT + an empty step for a lock with no note; COPY for steps and pages.
- Step edits made in place on the control half, with a device measurement of the worst period (Gate 0; [time threads](time-threads.md)).
- Full step cells and the held step's inspector on Beats; synth tracks on the step pads (the pen: a note or chord).
