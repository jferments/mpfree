<!--
covers: rust/mpfree-seq/src/notes.rs rust/mpfree-time-ui/src/editor.rs rust/mpfree-time-ui/src/view.rs rust/mpfree-app/src/time.rs
checked: 2026-11-12 efabc06
-->

# The note editor: piano roll, list and Timing Correct

**Status:** built and tested on the workstation, 2026-11-12. Nothing here has run on the MPC.

The note editor shows one clip's notes on a piano roll or as a list, on the Arrange page, so notes can be picked, moved, drawn, erased, nudged, transposed and timing-corrected. It is MPFree's take on the MPC's Grid View, List Edit and Timing Correct. The model is in `rust/mpfree-seq/src/notes.rs`, the screen in `rust/mpfree-time-ui/src/editor.rs`, and the pad player's side in `TimeHost::edit` (`rust/mpfree-app/src/time.rs`). The step grid on the Play page ([steps.md](steps.md)) is the other way to edit a clip; both make the same kind of edit.

## The model (`mpfree_seq::notes`)

Nothing here changes a clip. Every function returns a `steps::Edit` (the notes to take out and the notes to put in), which the control thread applies as one undoable take, as the step editor's edits are. A **selection** is a list of indexes into `clip.notes` as it is now. The edit names the notes themselves, so it applies and undoes whatever order the notes are kept in.

- **Timing Correct** (`Tc`, `timing_correct`), on all the notes, the selected ones, or a range of ticks and notes (`Which`):
  - **Start** moves each note's start toward the nearest line of the division (its length stays). **End** moves its end (its start stays). **Length** makes its length a whole number of divisions. **Legato** holds each note until the next one starts.
  - **Division**: the 12 entries of `DIVISIONS`, from 1 bar to 1/64, with triplets.
  - **Swing** (50 to 75 %) moves every second line later: 50 is straight, about 66 a triplet feel.
  - **Window** (0 to 100 %) corrects only notes that close to a line (100: every note). **Strength** (0 to 100 %) is how far of the way they move. **Shift** then moves every chosen note by that many ticks.
  - A start that comes to the clip's end wraps to its start, as the clip goes round.
  - `Tc::correct` gives one tick's corrected place, for recording with Timing Correct on (not wired yet).
- **Edits of a selection**: `nudge` (by a tick, or to the snap's lines: absolute, relative or no snap, `Snap`), `shift` (a drag: ticks and notes), `edit_start`, `edit_end`, `transpose`, `velocity` (by steps), `set_velocity`, `delete`, `add`, `duplicate` (after the selection, on the snap), `paste`, `split` (into equal parts), `humanize` (seeded, so the same seed gives the same result), and `pitch_quantize` (to a scale mask).
- **Whole-clip edits**: `double_length` (copies the notes, locks and FX moves), `half_speed` and `double_speed` (refused for a clip with FX moves, which can't be stretched yet).
- **Locks go with their notes.** A lock on a note's own sound and tick moves when the note moves and goes when it is deleted, and a duplicate copies it. Locks on a whole step (no sound) stay on their line, as the step editor keeps them.

## The screen (`mpfree_time_ui::editor`)

**Edit ›** at the top of the clip grid opens the editor on the clip the Play page's steps show (the armed track's launched clip, or its first clip). The editor takes the grid's place, and the Edit options list takes the Record list's place (`EDIT_LAYOUT`). The takes list stays, so each edit shows as a take.

- **`Editor`** is the state: the clip's track and slot, the selection, roll or list, the tool, the snap, the Timing Correct settings, the option picked, and where the roll or list is scrolled to. It lives in `TimeHost` and goes to the screen in `TimeState::edit` (`EditInfo`, with a copy of the clip).
- **`Editor::act`** turns an `EditAction` into a `Done`: the edit, a join key (so the wheel's steps of one option make one take), the log line, and how the selection follows. The app applies the edit through the take history, then calls `Editor::applied`, which finds the notes the selection should hold in the clip as it now is.
- **The piano roll** (`NoteGrid`): a key column, a ruler with bar numbers, the notes as bars (the selected ones lighter), the playhead, and a velocity lane under it. A pad clip (every note below 16) shows the 16 pads instead of keys. The roll starts at the C under the clip's lowest note, or at the lowest note when that would leave the highest notes off the roll (`ROLL_ROWS`). SHIFT + wheel scrolls the roll, or the list.
- **The list**: one row per note in time order (#, position as bar.beat.tick, note name, velocity, length, chance), the selected rows lit.
- **Touches**: a note picks it (a tap on a note already in the selection keeps the group, so a drag moves them all); a drag moves the selection by ticks and notes, snapped; a tap on empty space draws a note with the Draw tool or clears the selection with Select; the velocity lane sets the selection's velocity.
- **The buttons**: ‹ Clips (close), List/Grid, the tool (Select, Draw, Erase), the snap (13 settings: the divisions and off), All/None, Duplicate, Delete, and TC (TC sel. with notes selected, else TC all).
- **The Edit options**: Correct, Division, Swing, Window, Strength, Shift (Timing Correct's settings), then Nudge, Transpose, Velocity and Length, which act on the selection a step at a time. The wheel steps the option picked.
- **Selection after an edit**: Duplicate and Draw pick the new notes; a move, nudge, transpose, velocity or length change keeps the moved notes picked; Timing Correct leaves the selection as it was (none after correcting them all).

## Tests

- `mpfree-seq` (`notes.rs`, 10 tests): Timing Correct (start with the wrap at the end; end, length and legato; strength, window, swing and shift; the selection or a range), nudges with each snap mode, the grid edits, the whole-clip edits, humanize, pitch quantize, and locks going with their notes.
- `mpfree-time-ui` (`editor.rs`, 10 tests): a drag moving the selection and keeping it (and where the roll starts); draw, erase, duplicate and delete; Timing Correct on the selection or everything; the options' steps joining into one take; the settings buttons and scrolling; note names and the packed drag; the roll's touches; a drag snapping; the velocity lane and the list; painting both views.
- `mpfree-app`: `the_note_editor_edits_the_focused_clip_through_the_take_history` opens the editor through `TimeHost`, corrects all the notes, deletes a picked one and undoes both; a `gui.rs` test taps **Edit ›**, a note on the roll and TC.
- `mpfree --preview` draws `arrange-notes` (the roll) and `arrange-notes-list` (the list) for the manual.

## Not yet

- Buttons for split, humanize, pitch quantize, double length, half and double speed, copy and paste: the model has them.
- Timing Correct while recording (`Tc::correct`), and a range (`Which::Range`) picked on screen.
- Editing locks and FX moves in the roll; they move with their notes but are not shown.
- Pinch zoom on the roll; it shows the whole clip.
