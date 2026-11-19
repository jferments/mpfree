<!--
status: changing
covers: rust/mpfree-time-ui/src/editor.rs rust/mpfree-seq/src/notes.rs rust/mpfree-app/src/time.rs
checked: 2026-11-19 f0b39161
-->

# Editing notes

The note editor shows a clip's notes so you can fix what you played: pull notes into time, move them, change how hard they hit, draw new ones and erase others. It is the **Sequencer** tab's **Notes** view: tap **Notes** at the right end of the Sequencer's transport bar, or **Edit ›** over the clip grid on Arrange, which takes you there. It opens the clip the Sequencer's Steps show, which is the armed track's playing clip, or its first clip.

{{shot: sequencer-notes | The note editor's piano roll, with the melody picked and the Edit options beside it. Two long melody notes show the pressure they were played with.}}

The Notes view has the Arrange tab's transport bar along the top (**Stop**, **Play** and **Rec**, the record mode, the position, the tempo, the time signature, the launch line, **Undo** and the REC chip), the roll in the middle, the **Edit** options on the right with the clip's **Takes** under them, and a row of buttons under the roll:

| Button | What it does |
|---|---|
| **‹ Steps** | Back to the Steps view (so does **STEP SEQ**). |
| **Grid** | Which view shows: Grid, List or Tracker. Tap it for the next. |
| **Select** | The tool: Select, Draw or Erase. Tap it for the next. |
| **Snap 1/16** | The grid that moves and new notes land on. |
| **All** or **None** | Picks every note, or lets them go. |
| **Duplicate** | Copies the picked notes to just after them. |
| **Delete** | Takes the picked notes out (in Perform, hold it). |
| **TC sel.** or **TC all** | Timing Correct: pulls the picked notes (or, with none picked, every note) toward the grid. |

## The piano roll

The notes are bars on a piano roll: higher notes higher up, time running left to right, longer notes longer. A drum clip shows its 16 pads instead of keys. Under the roll, the velocity lane shows how hard each note plays. A note played with [pad pressure](clips.md#pad-pressure) shows it as a lighter shape along its bottom: it rises where you pressed harder and falls where you let up.

- **Pick notes.** Tap a note to pick it. **All** picks every note, and **None** lets them go.
- **Move them.** Drag a picked note and every picked note moves with it, along time and up or down in pitch. Moves land on the snap's lines (**Snap**, 1/16 to begin with).
- **Velocity.** Slide a finger along the velocity lane to set the picked notes' velocity.
- **Draw and erase.** The tool button switches between Select, Draw and Erase. With Draw, tap an empty spot to add a note there. With Erase, tap a note to take it out.
- **Duplicate** copies the picked notes to just after them. **Delete** takes them out (in Perform, hold it for half a second).
- **Grid, List, Tracker.** The second button says which view is showing (**Grid** to begin with) and switches to the next. **List** shows the notes as a list, one row per note, with its position, note, velocity, length and chance. **Tracker** shows every step, described below.

{{shot: sequencer-notes-list | The same notes as a list, the picked ones lit.}}

Every edit is a take, so **UNDO** takes it back and **SHIFT + UNDO** puts it back again. Tap **‹ Steps**, or press **STEP SEQ**, to close the editor and see the clip as steps again. The editor stays open while you visit other tabs, and the Sequencer tab shows it again when you come back.

## The tracker

The tracker shows the clip as a list of steps from top to bottom, empty steps included, the way a tracker does. It is handy for drum clips, where you want to read every step's settings at a glance.

{{shot: sequencer-notes-tracker | The tracker on a bar of drums: a nudged hat, a 1:2 hat, a FILL open hat, a ratchet, and the picked snare with a lock. The Euclid rows are beside it.}}

- **One row per step**, at the snap's size (1/16 to begin with). A step with several notes, such as a kick and a hat together, gives each note a row of its own under the step. An empty step shows dots.
- **The columns**: the step (bar.beat.step), the pad or note, velocity, length, nudge (how many ticks early or late), chance, condition, ratchet, and how many parameter locks the step has. A setting left at its default shows as a faint dot, so what you changed stands out.
- **Tap a note's row** to pick it, then change it with the Edit options. With the Draw tool, tap an empty step to put a note there: the sound of the last note you tapped, a step long. With Erase, tap a note's row to take it out.
- The step on each beat is shaded, a bar starts with a line, and a green mark shows the step playing.

With the snap off, or finer than 1/32, the tracker uses 1/16 steps, and its title line says so. The names of the parameters a step locks are coming; for now it shows how many there are.

## Timing Correct

Timing Correct pulls notes toward the beat, so a part played a little loose sits in time. Tap **TC sel.** and it corrects the picked notes; with none picked the button reads **TC all** and corrects every note. The MPC's **TC** button gets you here from any page: it opens the note editor on the track you are on, and pressed again it corrects, as **TC sel.** or **TC all** would. Its settings are the first rows of the Edit options beside the roll. Tap a row and turn the data wheel to change it:

- **Correct**: what moves. **Start** moves each note's start onto the grid. **End** moves its end. **Length** makes its length a whole number of grid steps. **Legato** holds each note until the next one starts.
- **Division**: the grid, from a whole bar to 1/64, with triplets.
- **Swing**: 50 % is straight. Higher values push every second step later, for a shuffle.
- **Window**: only notes this close to a grid line are corrected. At 100 %, every note is.
- **Strength**: how far of the way each note moves. At 100 % it lands on the line. Lower values keep some of the feel.
- **Shift**: moves the corrected notes a little early or late afterwards.

The next rows act on the picked notes straight away: **Nudge** moves them a step, **Transpose** a semitone, **Velocity** up or down, **Length** longer or shorter, **Chance** 5 % at a time, **Condition** through the trig conditions (Always, 1:2 to 8:8, then FILL, !FILL, PRE, !PRE, NEI, !NEI, 1ST and !1ST), and **Ratchet** from 1 to 8 hits. A run of turns on the same row is one take, so one **UNDO** takes it all back.

## Euclidean fill

The Euclidean fill spreads a number of hits as evenly as they go over a number of steps, a quick way to write a groove: 4 hits over 16 steps is four on the floor, 3 over 8 is the tresillo, 5 over 8 the cinquillo. It writes one sound's row, and its four rows are the last of the Edit options:

- **Euclid**: the sound it writes. It follows the last note you tapped; turn it to pick another pad or note, and that sound's notes light up.
- **Hits**: how many hits.
- **Steps**: how many steps the pattern spans (up to 64), at the snap's size, for example "16 × 1/16".
- **Rotate**: turns the pattern so it starts later.

Each turn of Hits, Steps or Rotate writes the pattern straight away, repeated through the whole clip. Steps that keep a hit keep their notes as they were; new hits are a step long and as loud as the sound's first note. Other sounds stay as they are. A pattern of 12 steps on a 16-step clip runs on into the next bar, for a part that drifts against the others. A run of turns is one take, so one **UNDO** puts the row back as it was.

> [!MPC]
> This is the MPC's Grid View, List Edit and Timing Correct in one place. TC here corrects notes already recorded, and the **TC** button opens it as on the MPC. To correct notes as you record them, set **Quantise** on the Record tab (see [Recording a clip](clips.md#recording-a-clip)), or press **SHIFT + TC**, the MPC's TC On/Off. The tracker and the Euclidean fill aren't on the MPC: the fill is like the Euclidean mode of Elektron's Digitakt.

