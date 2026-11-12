<!--
covers: rust/mpfree-seq/src/notes.rs rust/mpfree-seq/src/steps.rs rust/mpfree-time-ui/src/editor.rs rust/mpfree-time-ui/src/view.rs rust/mpfree-app/src/time.rs
checked: 2026-11-12 ea72d6c
-->

# The note editor: piano roll, list, tracker and Timing Correct

**Status:** built and tested on the workstation, 2026-11-12; the tracker lens and the Euclidean fill came later the same day. Nothing here has run on the MPC.

The note editor shows one clip's notes on the Arrange page through one of three lenses: a piano roll, a list, or a tracker. Notes can be picked, moved, drawn, erased, nudged, transposed and timing-corrected, and a step's chance, trig condition and ratchet set; the Euclidean fill writes a rhythm into one sound's row. It is MPFree's take on the MPC's Grid View, List Edit and Timing Correct, with a tracker's view of the steps and the Digitakt's Euclidean mode. The model is in `rust/mpfree-seq/src/notes.rs`, the screen in `rust/mpfree-time-ui/src/editor.rs`, and the pad player's side in `TimeHost::edit` (`rust/mpfree-app/src/time.rs`). The step grid on the Play page ([steps.md](steps.md)) is the other way to edit a clip; both make the same kind of edit.

## The model (`mpfree_seq::notes`)

Nothing here changes a clip. Every function returns a `steps::Edit` (the notes to take out and the notes to put in), which the control thread applies as one undoable take, as the step editor's edits are. A **selection** is a list of indexes into `clip.notes` as it is now. The edit names the notes themselves, so it applies and undoes whatever order the notes are kept in.

- **Timing Correct** (`Tc`, `timing_correct`), on all the notes, the selected ones, or a range of ticks and notes (`Which`):
  - **Start** moves each note's start toward the nearest line of the division (its length stays). **End** moves its end (its start stays). **Length** makes its length a whole number of divisions. **Legato** holds each note until the next one starts.
  - **Division**: the 12 entries of `DIVISIONS`, from 1 bar to 1/64, with triplets.
  - **Swing** (50 to 75 %) moves every second line later: 50 is straight, about 66 a triplet feel.
  - **Window** (0 to 100 %) corrects only notes that close to a line (100: every note). **Strength** (0 to 100 %) is how far of the way they move. **Shift** then moves every chosen note by that many ticks.
  - A start that comes to the clip's end wraps to its start, as the clip goes round.
  - `Tc::correct` gives one tick's corrected place, for recording with Timing Correct on (not wired yet).
- **Edits of a selection**: `nudge` (by a tick, or to the snap's lines: absolute, relative or no snap, `Snap`), `shift` (a drag: ticks and notes), `edit_start`, `edit_end`, `transpose`, `velocity` (by steps), `set_velocity`, `chance` (by percent, kept within 1 to 100), `condition` (each note's trig condition stepped along `Cond::all`, see [steps.md](steps.md#trig-conditions-cond)), `ratchet` (kept within 1 to 8), `delete`, `add`, `duplicate` (after the selection, on the snap), `paste`, `split` (into equal parts), `humanize` (seeded, so the same seed gives the same result), and `pitch_quantize` (to a scale mask).
- **Whole-clip edits**: `double_length` (copies the notes, locks and FX moves), `half_speed` and `double_speed` (refused for a clip with FX moves, which can't be stretched yet).
- **Locks go with their notes.** A lock on a note's own sound and tick moves when the note moves and goes when it is deleted, and a duplicate copies it. Locks on a whole step (no sound) stay on their line, as the step editor keeps them.

## The screen (`mpfree_time_ui::editor`)

**Edit ›** at the top of the clip grid opens the editor on the clip the Play page's steps show (the armed track's launched clip, or its first clip). The editor takes the grid's place, and the Edit options list takes the Record list's place (`EDIT_LAYOUT`). The takes list stays, so each edit shows as a take.

- **`Editor`** is the state: the clip's track and slot, the selection, the lens (`Lens`: Grid, List or Tracker), the tool, the snap, the Timing Correct settings, the option picked, where the roll, list or tracker is scrolled to, the **sound** being worked on (the last note tapped or drawn; to begin with the clip's lowest sound, else pad 1 or C3), and the Euclidean pattern (`Euclid`). It lives in `TimeHost` and goes to the screen in `TimeState::edit` (`EditInfo`, with a copy of the clip).
- **`Editor::act`** turns an `EditAction` into a `Done`: the edit, a join key (so the wheel's steps of one option make one take), the log line, and how the selection follows. The app applies the edit through the take history, then calls `Editor::applied`, which finds the notes the selection should hold in the clip as it now is.
- **The piano roll** (`NoteGrid`): a key column, a ruler with bar numbers, the notes as bars (the selected ones lighter), the playhead, and a velocity lane under it. A pad clip (every note below 16) shows the 16 pads instead of keys. The roll starts at the C under the clip's lowest note, or at the lowest note when that would leave the highest notes off the roll (`ROLL_ROWS`). SHIFT + wheel scrolls the roll, or the list.
- **The list**: one row per note in time order (#, position as bar.beat.tick, note name, velocity, length, chance), the selected rows lit.
- **The tracker**: one row per step, empty steps included, so the clip reads top to bottom as a tracker's pattern does (`track_rows`, `TrackRow`). The steps are the snap's (`Editor::grid`, `step_grid`); with the snap off or finer than 1/32, which a step grid can't hold, they are 1/16s, and the legend says so ("tracker · 1/16 steps (snap Off)"). A step's notes follow on rows of their own, lowest sound first; a note belongs to the step whose line is nearest, as on the step grid (`Grid::place`). The columns: the step as bar.beat.step (the step only when steps are shorter than a beat), the note or pad, velocity, length, nudge in ticks from the line, chance, condition, ratchet and the number of locks (the row's own on its step, and the whole step's on its first row; an empty step shows its trigless locks). A setting at its default shows as a faint dot, so what is set stands out. Rows are 26 px, so about a bar of 1/16s shows (less when steps hold several notes), and the whole row's width picks it; if taps land on the wrong row on the MPC's screen, they go to 30-32 px; the step on a beat is shaded, a bar starts with a line, and the playing step has a green mark. SHIFT + wheel scrolls it, and its hint line says so (`edit_hint`: the roll, the list and the tracker each have their own). Opening it scrolls to the first picked note.
- **Touches on the tracker**: a note's row picks it (Erase takes it out); an empty step's row gives `Cell { row: PEN, col: tick }` (`EditAction::Step`), which with Draw puts the editor's sound there, a step long, and with Select lets the notes go.
- **Touches**: a note picks it (a tap on a note already in the selection keeps the group, so a drag moves them all); a drag moves the selection by ticks and notes, snapped; a tap on empty space draws a note with the Draw tool or clears the selection with Select; the velocity lane sets the selection's velocity.
- **The buttons**: ‹ Clips (close), the lens (it shows the lens on screen, Grid, List or Tracker, lit when not Grid, and a tap goes to the next), the tool (Select, Draw, Erase), the snap (13 settings: the divisions and off), All/None, Duplicate, Delete, and TC (TC sel. with notes selected, else TC all).
- **The Edit options**: Correct, Division, Swing, Window, Strength, Shift (Timing Correct's settings), then Nudge, Transpose, Velocity, Length, Chance (5 % a step), Condition and Ratchet, which act on the selection a step at a time, then the Euclidean fill's four rows. The wheel steps the option picked; the list scrolls to keep it in view.

### Euclidean fill

The last four Edit options (from `EUCLID_ROW`) write a Euclidean rhythm into one sound's row ([steps.md](steps.md#euclidean-fill-euclid-grideuclid)):

- **Euclid** is the sound it writes: the editor's sound (the last note tapped). Turning it steps the pad or note and picks that sound's notes, so the roll shows which; it writes nothing.
- **Hits**, **Steps** (shown as "16 × 1/16": the steps and their size, the tracker's) and **Rotate** set the pattern. Each turn writes it at once (`Grid::euclid`), as the Nudge and Transpose rows act at once, and picks the sound's notes after (`Done::select_sound`). The three rows share one join key, so a run of turns is one take and one UNDO puts the row back as it was.
- New hits play as loud as the sound's first note (100 with none) and are a step long. Hits that land on notes already there keep them.
- **Selection after an edit**: Duplicate and Draw pick the new notes; a move, nudge, transpose, velocity or length change keeps the moved notes picked; Timing Correct leaves the selection as it was (none after correcting them all).

## Tests

- `mpfree-seq` (`notes.rs`, 10 tests; chance, condition and ratchet in the grid edits' test): Timing Correct (start with the wrap at the end; end, length and legato; strength, window, swing and shift; the selection or a range), nudges with each snap mode, the grid edits, the whole-clip edits, humanize, pitch quantize, and locks going with their notes.
- `mpfree-time-ui` (`editor.rs`, 13 tests): a drag moving the selection and keeping it (and where the roll starts); draw, erase, duplicate and delete; Timing Correct on the selection or everything; the options' steps joining into one take; the settings buttons, the three lenses and scrolling; note names and the packed drag; the roll's touches; a drag snapping; the velocity lane and the list; painting every lens. The tracker: its rows (every step, a step's notes on rows of their own) and cells (positions, nudge, chance, condition, ratchet, locks and trigless locks), its touches and scrolling, and opening on the picked note. The sound and the Euclidean rows: Draw on an empty step, Hits and Rotate writing the pattern as one take with the sound's notes picked, the Euclid row picking a sound without writing, the Chance, Condition and Ratchet rows, and 1/16 steps with the snap off.
- `mpfree-seq` (`steps.rs`): Euclidean patterns and the fill; see [steps.md](steps.md#tests).
- `mpfree-app`: `the_note_editor_edits_the_focused_clip_through_the_take_history` opens the editor through `TimeHost`, corrects all the notes, deletes a picked one and undoes both; a `gui.rs` test taps **Edit ›**, a note on the roll and TC.
- `mpfree --preview` draws `arrange-notes` (the roll), `arrange-notes-list` (the list) and `arrange-notes-tracker` (the tracker on a bar of drums with a nudge, a 50 % ghost note, a 1:2 hat, FILL, a ratchet and a lock, the Euclidean rows on show) for the manual.

## Not yet

- Buttons for split, humanize, pitch quantize, double length, half and double speed, copy and paste: the model has them.
- Timing Correct while recording (`Tc::correct`), and a range (`Which::Range`) picked on screen.
- Editing locks and FX moves in the roll; they move with their notes but are not shown. The tracker counts a step's locks but doesn't name them yet (the parameter names live with the track's instrument, not in `mpfree-seq`).
- A tracker cursor that the wheel moves cell by cell with data entry from the pads, and the tracker following the playhead.
- A second Euclidean pattern per sound (the Digitakt's two pulse generators) and Euclidean fills of several sounds at once.
- Pinch zoom on the roll; it shows the whole clip.
