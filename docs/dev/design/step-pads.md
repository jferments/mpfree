<!--
covers: rust/mpfree-app/src/app/step_pads.rs rust/mpfree-app/src/step_keys.rs rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/play_widgets.rs rust/mpfree-app/src/locks.rs rust/mpfree-seq/src/steps.rs
checked: 2026-11-17 dc4da93
-->

# STEP SEQ: the pads as steps

**Status:** S1 of the sequencer roadmap: the pads as steps (first part), then holding a step, ties and COPY (second part), 2026-11-16; locks (third part), 2026-11-17; step edits in place (fourth part), 2026-11-17; full step cells and the step inspector on Beats (fifth part), 2026-11-17; the synth's steps, the line and its pen (sixth part), 2026-11-17; a synth step's notes, the Note knob and a keyboard's keys (seventh part), 2026-11-17; scale rows on Beats for a synth track (S3, first part), 2026-11-17. Decided by the owner on 16 Nov 2026 (sequencer decision 2, card "Pads become steps, as designed", 20:00Z). The design notes, with mockup I (`step-pads.png`) and the Beats mockup, are in the private notes repository (`sequencer/rethink/README.md`, "STEP SEQ: the pads as steps"). The buttons and the screen are in the [layout spec](layout-spec.md) (sections 2, 4 and 6). Tested on the host; not yet run on the MPC (device checks `step-pads`, `step-hold`, `step-locks`, `step-synth`, `step-notes`, `step-keys` and `step-speed`).

This builds on the [step sequencer](steps.md): steps are a view of a clip's notes, and each edit is one take.

## The rule

**The lit mode button says what the pads are.** With STEP SEQ lit, the 16 pads are 16 steps of one sound of the clip Beats shows (the armed track's playing clip, else its first), on every page, until STEP SEQ is pressed again. TRACK MUTE works the same way, and there is one pad mode at a time: STEP SEQ, TRACK MUTE and 16 LEVEL each turn the others off.

| Press | Does |
| --- | --- |
| STEP SEQ (on its release) | The pads become steps, and the Sequencer tab shows its Steps (Beats' pages; the note editor closes); again, they play |
| Hold STEP SEQ, tap a pad | The pads are the kit while it is held (PAD BANK picks the kit's bank); the pad tapped sounds and is the sound when STEP SEQ is let go |
| SHIFT + STEP SEQ | Follow: the page turns with the playhead |
| Tap an empty step | A note at the pad's velocity (127 with FULL LEVEL), heard once while the transport is stopped |
| Tap a lit step | It comes off, with its locks, when the pad is let go, if it was the only step held and nothing was done while it was |
| Hold a step | The Q-Links on Play set it: its Step bank (velocity, length, nudge, chance); Q-LINK steps to its Trig bank (condition, ratchet, nudge in ticks), then its Sound bank, and back. A finger on a Q-Link shows the step's value on its card |
| Hold a step, turn a Q-Link on the Sound bank | A lock on the steps held: a kit pad's Tune, Decay, Filter and Level, or the synth's first Q-Link set. The card shows the lock in purple |
| SHIFT + an empty step | It is held with no note, and the Q-Links show the Sound bank: a turn is a lock with no note |
| Hold a step, turn a knob on the Synth tab | When the step's track plays the synth the tab shows: a lock on the steps held, not a change to the sound. The tab shows the step's locks while it is held |
| Hold a step, press ERASE | The steps held lose their locks; their notes stay |
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
- **The Q-Links** (`play_page.rs`): with steps held, `PlayPage::knobs` is the Step bank, or after Q-LINK the Trig bank, then the Sound bank (`PlayPage::bank`, a `HeldBank`), on every Play view and on Arrange, over a macro bank; steps held with no note show the Sound bank only (`held_bank`). A turn changes the step held first that has a note; with several held it sends `ChangeSteps` with all of them, at that step's new value. The cards' scope and the column's title name the steps ("Q-Links · Steps 5, 13 · Trig"). Letting go brings the page's own knobs back, and the next hold starts on the Step bank.
- **COPY + PAD BANK** is the app's (`App::step_page_button`): the page that button lights (SHIFT: the other four), as ERASE + PAD BANK, picks the page, and each PAD BANK after takes a copy, until COPY is let go.

Holding a step on another page and turning a knob there does what the knob does on that page, except on the Synth tab (below); the mixer's and the effects' knobs become locks in a later part.

## Locks

A lock is a value a step's sound plays with in place of its own. Clips kept them and the player played them already (`Event::Lock` and `Event::Unlock`, [playback.md](playback.md#parameter-locks)), but only the mixer's strips and inserts took one (`modmix.rs`). The third part gives a pad's sound and the synth their locks, and the ways to set them.

- **Where a lock goes.** The audio loop gives each `Lock` and `Unlock` to the pads first (`time::Pads::lock_at`; `SeqPads` in `main.rs`, `Hits` in the smoke session). `Owner::Pad` on a kit's track goes to the sampler engine (`sampler::Player::pad_lock`, `Event::PadLock`); the lock's parameter is the `PadField` number, its value in the field's unit: Level 0 (dB), Pan 1 (−1 to 1), Tune 2 (cents), Cutoff 3 (Hz), Resonance 4 (0 to 1), Attack 5, Decay 6 and Release 7 (ms). `Owner::Inst` goes to the track's own instrument (`Instruments::lock`), or, on a track that plays the synth, to the synth (`Synth::lock`). `Owner::Pad` goes to the kit its track plays. One none of them takes (a MIDI-out track's, the mixer's) goes on to the mod engine as before.
- **What its end comes back to.** The synth and the instruments keep the value under each lock (`locks::Under`, 64 parameters each): a run of locked steps comes back to the sound's own value, not an earlier step's; a knob turned while a lock holds is what its end comes back to; and an end and a new lock on the same frame (a clip switch) still find the sound's own. The sampler keeps locks in a table of their own under the pad's live values, so it needs none.
- **When it ends.** The player's rule: a pad's lock holds from its step's frame until that pad's next note without it; a lock with no note holds the same way; a stop, a locate or a loop jump ends them all.
- **How it sounds.** A kit pad's Level, Pan, Filter and Resonance move the sound still ringing; Tune, Attack, Decay and Release reach the next note. A locked Decay decays to silence, as the Play page's Decay knob does. The scene crossfader goes over a lock.
- **Setting one** (`play_page.rs`, `gui.rs`). `StepAction::Lock { pad, steps, addr, value }` puts a lock on every step held, with a note or not (`Grid::lock_steps`); a turn of one knob over the same steps is one undo (`lock_key_steps`). `StepAction::ClearLocks` takes the held steps' locks off and leaves their notes (`Grid::clear_step_locks`). `StepClip::lock_values` gives the screen each row's locks, so a Sound bank card shows the held step's lock, in purple, or the sound's own value. The Sound bank (`PlayPage::sound_bank`, filled by `held_sound_bank` in `gui.rs`) is a kit pad's Tune, Decay, Filter and Level, read from the Sample tab's pad (`SamplePage::pad_sound`), or the synth's first Q-Link set (`qsets::factory`, `qsets::on_pad` for Drums); a pad with neither shows empty cards.
- **The Synth tab.** With steps held on a track that plays the synth the tab shows (its own instrument, or the synth on the pads), a knob turned there goes out as a `StepAction::Lock` on the steps held instead of a change to the sound (`as_held_lock` in `gui.rs`, on every action the screen sends). While they are held the tab shows the first step's locks, and once they are let go its own values again (`held_synth_values`).
- **SHIFT + an empty step** holds it with no note (`Press::shift`). **ERASE pressed while steps are held** sends `ClearLocks` for them (`erase_held_steps` in `main.rs`, before the app's own ERASE, and `StepKeys::erase_locks`).

## On the synth: the line and the pen

On a melodic track the design makes the pads the steps of the whole line, chords included, as one lane does on the KeyStep Pro, and a new step plays the **pen**: the note or chord picked last. Here a melodic track is the synth on the pads (`App::set_pad_layout` has its layout). Its clips keep pad numbers, as every track on the pads does, and the synth plays each through its pad layout, so a pad of the Chords layout is a whole chord and the pen holds pads, not notes.

- **The line** (`time::LINE`, `mpfree_seq::steps::LINE`, a row number no note has): `StepPads::sound` is it while the synth has the pads (`StepPads::line`). `StepClip` carries the line as a row of its own (`Grid::line`, each step's notes as one `Step` with their count) with its lock counts (`Grid::lock_line`), and a whole step's locks (a lock with no sound) show in `lock_values` on the line's row. Every step action takes `LINE` as its pad: `Change` is `Grid::change_all`, `ChangeSteps` `change_line`, `Lock` and `ClearLocks` the whole step's (no sound), `ClearSteps` `clear_line` and `CopySteps` `copy_line`, so ties, ERASE, COPY, the Q-Links and locks work as on a sound's steps, on every note of a step at once.
- **The pen** (`step_pads::Pen`, up to `MAX_PEN`, 8 pads): STEP SEQ held, the pads play the synth as usual and `App::step_pad_played` writes the pen: a pad played with none of the others down starts a new pen, and pads held with it join it (a chord); `App::pad_up` lets them go (`step_pad_let_go`). With no pen yet, it is the pad played last. `App::pen_name` names its notes ("C3 Eb3 G3") for the lines; the chip shows the synth's name, then its pads' names joined with + ("Pads: steps · Poly · C3 + G3").
- **A press** (`StepKeys::press`): an empty step sends `StepAction::Enter { step, keys, n }`, the pen's pads at the press's velocity (`Grid::enter`: in place of what the step held, a step long; with no clip, one is made and launched as `Tap` does), and while stopped every pad of the pen sounds once (`Pressed::sound`, a list, as `Released::silence`). A lit step pressed alone comes off whole on its release (`ClearSteps` of the line). With no pen, a press says to hold STEP SEQ and play a pad.
- **The Note knob** (`play_page::Knob::Note`): on the line the Step bank is Note, velocity, length and chance, and the nudge is Trig's (in ticks), as the design has it for synth tracks (`PlayPage::step_bank`, `on_line`). A detent moves the step's notes a pad up or down (`Change::Note`: the chord keeps its shape and its rows' locks go with it), never past pad 16 for the widest chord set (`PlayPage::note_top`). The card is "Note", or "Chord" for several notes, with the lowest note's name; a detent is a line ("steps: step 1 of the line: lowest note on pad 3"). The inspector's title names a line step's notes ("Step 1 · E3 + G3", `PlayPage::line_notes`), and its chips outline velocity, length and chance on the Step bank.
- **A keyboard's keys** (`StepKeys::key_down`, `key_up`): with steps of the line held, each key's note goes to the pad whose note it is (a chord pad's lowest, `App::pad_of_key`), and the keys held together go on every step held, each at its own velocity, in place of what the step held (`StepAction::Keys`, `Grid::enter_steps`); the keys of a chord, and any chord after it while the steps stay held, join one undo (`steps::enter_key`). The steps stay on their release. A key no pad plays says so and changes nothing; with no step held a key just plays the synth. `main.rs`' `step_key` takes the keys from MIDI IN and USB keyboards as the synth does (`midi_in::Key`).
- **The screen.** Beats' lane on the pads is the line, named "Line" in its pen's colour; a tap on its empty step puts the pen there (`PlayPage::tap_line`), and on a lit one picks it, then takes it off. The hint line says how to pick the pen while STEP SEQ is held.

## Scale rows on Beats (S3)

On a synth track the pattern's lanes are notes, as the design's mockup B draws them: Beats (and the All view's pattern) shows **scale rows**, the synth's pads in pitch order with the highest on top, each named by its note (a chord pad by its chord), the key's root shaded and tagged "root" (with no scale, each C, as the pads' lights have it). The clip still keeps pad numbers, so a row is a pad, and a tap on its step is the same `StepAction::Tap` as on a sound's lane; a chord is several rows lit on one step.

- **Which rows** (`PlayPage::note_rows`): every pad with a note (`PadInfo::pitch`, from `KitView::pitches`, which `Synth::pad_pitches` fills with each pad's note, a chord's root, and whether its light is the root's; empty for a kit and the Drums engine, whose pads are sounds), sorted by note, any pad with steps but no note of its own below them. **Fold** on the pattern's head keeps only the rows the clip uses, or all of them while it uses none.
- **The window** (`PlayPage::low_row`, `play_widgets::Pitched`): the grid shows as many rows as fit (six on Beats with the timing strip, eight without it), from the root at or under the lowest note used, within an octave; with no notes, from the lowest root. The head's ‹ and › round "Oct 3" (`HEAD_LOWER`, `HEAD_HIGHER`) move it an octave down or up (`PlayPage::turn_octave`: to the row an octave from the lowest shown, stopping at the ends, which the hint line says); "Oct 3" names the lowest row's octave (48, C3, is octave 3). Fold, or another layout or scale on the pads, starts the window again from the notes. `StepGrid::shown` clamps it to the rows there are.
- **A row's name plays its note** while a finger is on it (`EventKind::Pad` down and up, `FromUi::Audition`: the synth's pad on and off in `main.rs`, never recorded and never a step press; the smoke session renders no sound). With ERASE held, a row's name clears that note's steps, as a sound's lane does. A synth's pads have no mutes of their own (the kit's pad mutes are the drums'), so the rows have no speaker.
- **The head** adds Fold and the octave before the page's ‹ and ›; the head's details ("1/16 · 1 bar", "pads: page 2 · BANK B") give way when they would crowd the title.
- **With STEP SEQ** the line's lane stays on top, above the window of notes, as the lane on the pads.

## The step inspector and the cells

The Beats mockup (A) shows a step whole, on its cell and under the pattern. Each lane carries the marks of its page (`play_widgets::Marks`, made by `play_page::marks` from the clip's row and `StepClip::locks`): condition, chance, ratchet, the nudge's side, a tie into the next step, the empty steps a note lasts over, and the lock count. `paint_cell` draws them, and the grid's paint cache (`StepLook`) compares the lanes with their marks, so a mark that changes is drawn again.

The inspector (`play_widgets::StepInspector`, filled by `PlayPage::inspect`) shows while a step is picked on Beats or held with STEP SEQ (`PlayPage::inspecting`). It takes the throws' place on two more pages, `Play · Beats · step` and `Play · Beats · timing · step` (`PlayView::page_with`, `PlayView::step_layout`), the same 196 px, so nothing else moves; `Player::refresh_play` swaps the page when the step comes and goes. Its taps (`PlayPage::inspector_tap`):

- **Step, Trig, Sound** set the bank the Q-Links show (`PlayPage::bank`), as Q-LINK does; a picked step has Step and Trig (Q-LINK steps between them, `HeldBank::Trig` giving `knobs()` the condition, ratchet and nudge in ticks), Sound only while held.
- **A lock's ×**: `StepAction::Lock` with no value for that parameter, on every step held or the step picked. Unlike a turn's detents, it joins no take (`TimeControl::step_action`), so UNDO brings the lock back.
- **Copy** keeps the step (`PlayPage::copy_from`); the next step tapped on the lane is `StepAction::CopySteps` with a count of 1 (locks along), and any other tap lets it go.
- **Clear**: `StepAction::ClearSteps` of that step, locks too.
- **×** (a step picked, not held): picks it off and keeps it (`InspectTap::Close`), so the throws and Q-LINK's macro banks come back.

## Speed: edits in place

Every press edits a clip that plays, so a tap or a turn must not cost the audio thread a compile of the whole view (a turn sends many detents). Since the fourth part, a tap, a change on a held step and a lock go into the playing view in place: the control half sends the notes and locks added and erased, and the audio half makes them where a compile would, without allocating ([time threads](time-threads.md#step-edits-in-place)). A new pattern, a length change, a big erase or copy, song mode and anything while recording still compile. The status line says what step edits and compiles cost, and the `step-speed` device check measures it on the MPC (Gate 0 of the roadmap).

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

- Locks from the mixer's and the effects' knobs turned on their own pages with a step held (the mod engine already plays them).
- Tracks with their own instrument on the step pads.
