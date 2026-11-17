<!--
covers: rust/mpfree-app/src/app/step_pads.rs rust/mpfree-app/src/step_keys.rs rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/play_widgets.rs rust/mpfree-seq/src/steps.rs
checked: 2026-11-16 0fc15c6
-->

# STEP SEQ: the pads as steps

**Status:** S1 of the sequencer roadmap: the pads as steps (first part), then holding a step, ties and COPY (second part), 2026-11-16. Decided by the owner on 16 Nov 2026 (sequencer decision 2, card "Pads become steps, as designed", 20:00Z). The design notes, with mockup I (`step-pads.png`) and the Beats mockup, are in the private notes repository (`sequencer/rethink/README.md`, "STEP SEQ: the pads as steps"). The buttons and the screen are in the [layout spec](layout-spec.md) (sections 2, 4 and 6). Tested on the host; not yet run on the MPC (device checks `step-pads` and `step-hold`).

This builds on the [step sequencer](steps.md): steps are a view of a clip's notes, and each edit is one take.

## The rule

**The lit mode button says what the pads are.** With STEP SEQ lit, the 16 pads are 16 steps of one sound of the clip Beats shows (the armed track's playing clip, else its first), on every page, until STEP SEQ is pressed again. TRACK MUTE works the same way, and there is one pad mode at a time: STEP SEQ, TRACK MUTE and 16 LEVEL each turn the others off.

| Press | Does |
| --- | --- |
| STEP SEQ (on its release) | The pads become steps, and Play shows Beats; again, they play |
| Hold STEP SEQ, tap a pad | The pads are the kit while it is held (PAD BANK picks the kit's bank); the pad tapped sounds and is the sound when STEP SEQ is let go |
| SHIFT + STEP SEQ | Follow: the page turns with the playhead |
| Tap an empty step | A note at the pad's velocity (127 with FULL LEVEL), heard once while the transport is stopped |
| Tap a lit step | It comes off, with its locks, when the pad is let go, if it was the only step held and nothing was done while it was |
| Hold a step | The Q-Links on Play set it: its Step bank (velocity, length, nudge, chance); Q-LINK steps to its Trig bank (condition, ratchet, nudge in ticks) and back. A finger on a Q-Link shows the step's value on its card |
| Hold several steps and turn | They all take the first one's new value, as one undo |
| Hold a step, tap a later one | Before any turn: the note lasts to the end of that step, or ends there if the tap is inside it. A lit step, an earlier one, or any step after a turn joins the steps held |
| COPY held + a step, then others | The first step is picked; each step tapped after takes a copy, locks included |
| COPY held + PAD BANK, then another | The page of the sound's steps is copied over the other page |
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
- **The engine** adds `Grid::clear_steps` (one sound's notes and locks on a range of steps) and `StepClip::locks` (each step's lock count, for the lock colour); `Grid::copy_steps` (one sound's steps over others, nudges and locks along, for COPY) and `Grid::change_steps` with `Change::key_steps` (several held steps changed as one edit, joining one undo for a whole turn), behind `StepAction::CopySteps` and `StepAction::ChangeSteps`.

## Holding a step

- **The step keys** (`StepKeys` in `step_keys.rs`) keep what each held pad is for: a step to set, or a press that was ERASE, COPY, a tie's end or past the clip's end. `held_steps` is the sound and the steps to set, the first held first; the audio loop sends it to the screen when it changes (`ToUi::StepsHeld`). A screen action that changes a step (`StepAction::Change` or `ChangeSteps`, from a Q-Link) calls `StepKeys::turned`, so the steps held stay on their release and a later tap joins them rather than tying. COPY held (`App::copy_held`) makes a press pick the step to copy, then copy it; COPY let go forgets it.
- **The tie** is `Change::Length` on the step held first: from its note to the end of the step tapped (its nudge taken off), so the lights' "held over" glow shows it at once.
- **The Q-Links** (`play_page.rs`): with steps held, `PlayPage::knobs` is the Step bank, or after Q-LINK the Trig bank, on every Play view and on Arrange, over a macro bank. A turn changes the step held first that has a note; with several held it sends `ChangeSteps` with all of them, at that step's new value. The cards' scope and the column's title name the steps ("Q-Links · Steps 5, 13 · Trig"). Letting go brings the page's own knobs back, and the next hold starts on the Step bank.
- **COPY + PAD BANK** is the app's (`App::step_page_button`): the page that button lights (SHIFT: the other four), as ERASE + PAD BANK, picks the page, and each PAD BANK after takes a copy, until COPY is let go.

Holding a step on another page and turning a knob there does what the knob does on that page; it becomes a lock with the next part.

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

- Locks: the lock targets that let a pad's sound and the synth play a lock (today only the mixer's strips and inserts take one, `modmix.rs`), then the Q-Links' Sound bank with a step held (the sound's tune, decay, filter and level), any knob turned on another page with a step held, and SHIFT + an empty step for a lock with no note.
- Step edits made in place on the control half, with a device measurement of the worst period (Gate 0; [time threads](time-threads.md)).
- Full step cells and the held step's inspector on Beats; synth tracks on the step pads (the pen: a note or chord).
