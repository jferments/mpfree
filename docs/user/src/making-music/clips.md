<!--
status: changing
covers: rust/mpfree-seq rust/mpfree-play rust/mpfree-capture rust/mpfree-time-ui
checked: 2026-11-12 efabc06
-->

# Clips and patterns

Clips hold what you play. Record them, keep takes, capture something you played without recording, and launch clips on the Arrange tab.

Most of this chapter is not written yet. It will cover:

- What a clip is
- Recording a clip, overdub and takes
- Capture: keep what you just played
- Steps: velocity, length, nudge, chance
- Launching clips

## Editing notes

The note editor shows a clip's notes so you can fix what you played: pull notes into time, move them, change how hard they hit, draw new ones and erase others. Tap **Edit ›** above the clip grid. It opens the clip the Play page's steps show, which is the armed track's playing clip, or its first clip.

{{shot: arrange-notes | The note editor's piano roll, with the melody picked and the Edit options beside it.}}

The notes are bars on a piano roll: higher notes higher up, time running left to right, longer notes longer. A drum clip shows its 16 pads instead of keys. Under the roll, the velocity lane shows how hard each note plays.

- **Pick notes.** Tap a note to pick it. **All** picks every note, and **None** lets them go.
- **Move them.** Drag a picked note and every picked note moves with it, along time and up or down in pitch. Moves land on the snap's lines (**Snap**, 1/16 to begin with).
- **Velocity.** Slide a finger along the velocity lane to set the picked notes' velocity.
- **Draw and erase.** The tool button switches between Select, Draw and Erase. With Draw, tap an empty spot to add a note there. With Erase, tap a note to take it out.
- **Duplicate** copies the picked notes to just after them. **Delete** takes them out.
- **List** shows the notes as a list instead, one row per note, with its position, note, velocity and length. Tap **Grid** to go back to the roll.

{{shot: arrange-notes-list | The same notes as a list, the picked ones lit.}}

Every edit is a take, so **UNDO** takes it back and **SHIFT + UNDO** puts it back again. Tap **‹ Clips** to close the editor.

### Timing Correct

Timing Correct pulls notes toward the beat, so a part played a little loose sits in time. Tap **TC** and it corrects the picked notes, or every note if none are picked. Its settings are the first rows of the Edit options beside the roll. Tap a row and turn the data wheel to change it:

- **Correct**: what moves. **Start** moves each note's start onto the grid. **End** moves its end. **Length** makes its length a whole number of grid steps. **Legato** holds each note until the next one starts.
- **Division**: the grid, from a whole bar to 1/64, with triplets.
- **Swing**: 50 % is straight. Higher values push every second step later, for a shuffle.
- **Window**: only notes this close to a grid line are corrected. At 100 %, every note is.
- **Strength**: how far of the way each note moves. At 100 % it lands on the line. Lower values keep some of the feel.
- **Shift**: moves the corrected notes a little early or late afterwards.

The last rows act on the picked notes straight away: **Nudge** moves them a step, **Transpose** a semitone, **Velocity** up or down, and **Length** longer or shorter. A run of turns on the same row is one take, so one **UNDO** takes it all back.

> [!MPC]
> This is the MPC's Grid View, List Edit and Timing Correct in one place. TC here corrects notes already recorded. Correcting while you record is coming.
