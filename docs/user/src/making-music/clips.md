<!--
status: changing
covers: rust/mpfree-seq rust/mpfree-play rust/mpfree-capture rust/mpfree-time-ui rust/mpfree-transport
checked: 2026-11-12 15df706
-->

# Clips and patterns

A **clip** is a piece of music you recorded or programmed: a drum pattern, a bass line, a chord part. The pattern on the Play page is a clip. Clips live on **tracks**, and the **Arrange** tab shows them all as a grid so you can record new ones, keep the best takes and switch between them while the music plays.

{{shot: clips | The Arrange tab: tracks across the top, clip slots down, the record settings and the takes beside them, and the Q-Links down the right edge.}}

## Tracks and slots

The grid has a column per **track** (up to 16) and a row per **slot**. Each track plays one clip at a time, so a column might hold four different drum patterns, and you pick which one plays.

- **Tap a clip** to launch it. It starts on the next bar line, so it always lands in time. Tap a playing clip to stop it at the next bar.
- **Tap +** in an empty slot to record a new clip there.
- **Tap a track's name** to **arm** it: the pads record into that track.
- **Hold ERASE and tap a clip** to delete it. **UNDO** brings it back. A clip the song uses stays until you delete its blocks in the song.

A clip's border tells you what it's doing: a ring going round as it plays one pass, amber while it waits for the next bar, red while recording.

## The Q-Links on Arrange

The four Q-Link cards run down the right edge of Arrange, in the same place as on the Play page, so the knobs never move between the two. They do the same as on Play: the knobs of the Play view you picked last, as the cards show (see [The Q-Links on the Play page](playing-live.md#the-q-links-on-the-play-page)). Turn a knob, or drag up or down on its card. **Q-LINK** steps the cards to Macros 1-4 and Macros 5-8, and Play and Arrange share the bank.

Coming:
- With an audio region picked in the Song view, the cards become the audio track's level and pan, and the region's gain and fade.
- A track with its own instrument has its own mixer channel (see [A synth for each track](synths.md#a-synth-for-each-track)), but its level and pan aren't on the knobs here yet. Tracks that play the kit share the kit's channels.

## Recording a clip

Set up the recording in the Record panel, beside the grid:

- **Armed:** the track that records.
- **Mode:** **Replace** records over what's there; **Overdub** adds to it. The **OVERDUB** button switches this too.
- **Count-in:** a count before recording starts, so you can come in on time.
- **Loop:** plays the same bars round and round while you record, so you can keep trying a part. It counts in bars from bar 1.
- **Tempo:** the song's tempo, in BPM.
- **Pressure:** whether the armed track records how hard you press the pads while they're held (**Recorded**, the setting for a new track, or **Off**). It's the last row: pick Loop or Tempo and the list moves up to show it.

Tap a row's arrows, or the left or right of its value, to change it. Or turn the data wheel to pick a row and press **–** or **+**. Push the wheel and turning it changes the row picked; push again to go back to picking rows.

Then press **REC**, play, and press **STOP**. The new notes go into the armed track's clip.

### Pad pressure

The pads feel how hard you keep pressing after you hit them. While you record, that pressure goes into the clip with each note, and the clip plays it back the way you played it: a synth swells, opens up or wobbles just as it did live. It works on drum pads too, where a sound's pressure settings use it.

- The pressure stays with its note. If the clip's quantise moves a note into time, or you move the note in the note editor, its pressure moves with it.
- MPFree keeps only the movement that matters: a change of two steps or more right away, a small change once it has lasted a moment (10 ms). A finger's tiny wobble isn't kept, so clips stay light.
- The clip keeps the raw pressure, and each sound decides what pressure does. Change a sound's pressure curve and the clip plays back through the new one.
- Splitting a note in the note editor keeps its pressure on the first part only.
- The note editor shows it: a note you leaned into has a lighter shape along its bottom, as high as you pressed (see [Editing notes](#editing-notes)).
- Capture keeps it too, and the song plays it wherever a clip is placed.
- Turn **Pressure** to **Off** for a track that shouldn't record it, for example drums you want to stay steady. Capture leaves it out on that track too.
- Coming: editing pressure in the note editor.

## Takes

Every recording is a **take**, listed under Takes. **UNDO** takes back the newest take, and **SHIFT + UNDO** puts it back, so you can try a part again and again without worrying about losing the last one.

## Capture: keep what you just played

You weren't recording, but you just played something great. Tap the **Capture** button under Takes (it says how many bars it would keep), or press **SHIFT + REC** in the next build, and MPFree keeps it.

MPFree is always listening to the pads. Capture keeps your **last phrase**: everything since the last bar of silence, up to 32 bars, cut to whole bars and with your exact timing.

- If you played it in time over a clip that was playing, the notes go into that clip.
- Otherwise it becomes a new clip, named "Capture 1", "Capture 2" and so on, and starts playing in time.
- It keeps how hard you pressed the pads as you held them, as recording does (see [Pad pressure](#pad-pressure)), unless the track's **Pressure** setting is **Off**.

UNDO takes a capture back like any other take.

> [!MPC]
> Capture is like the MPC's Retro Rec, but it keeps whole bars, works whether the music was playing or stopped, and can add to the clip that's playing instead of always making a new one.

## Steps

The pattern on the Play page's **Beats** view is a clip shown as a grid of steps, 16 to a page. Each step can have its own **velocity**, **length**, **nudge** (early or late) and **chance** (how often it plays). The **+** and **–** at the top right of the pattern double or halve its steps (16, 32, 64 or 128), and **‹** and **›** turn its pages. See [Your first beat](../getting-started/first-beat.md#program-a-pattern).

## Editing notes

The note editor shows a clip's notes so you can fix what you played: pull notes into time, move them, change how hard they hit, draw new ones and erase others. Tap **Edit ›** above the clip grid. It opens the clip the Play page's steps show, which is the armed track's playing clip, or its first clip.

{{shot: arrange-notes | The note editor's piano roll, with the melody picked and the Edit options beside it. Two long melody notes show the pressure they were played with.}}

The notes are bars on a piano roll: higher notes higher up, time running left to right, longer notes longer. A drum clip shows its 16 pads instead of keys. Under the roll, the velocity lane shows how hard each note plays. A note played with [pad pressure](#pad-pressure) shows it as a lighter shape along its bottom: it rises where you pressed harder and falls where you let up.

- **Pick notes.** Tap a note to pick it. **All** picks every note, and **None** lets them go.
- **Move them.** Drag a picked note and every picked note moves with it, along time and up or down in pitch. Moves land on the snap's lines (**Snap**, 1/16 to begin with).
- **Velocity.** Slide a finger along the velocity lane to set the picked notes' velocity.
- **Draw and erase.** The tool button switches between Select, Draw and Erase. With Draw, tap an empty spot to add a note there. With Erase, tap a note to take it out.
- **Duplicate** copies the picked notes to just after them. **Delete** takes them out.
- **Grid, List, Tracker.** The second button says which view is showing (**Grid** to begin with) and switches to the next. **List** shows the notes as a list, one row per note, with its position, note, velocity, length and chance. **Tracker** shows every step, described below.

{{shot: arrange-notes-list | The same notes as a list, the picked ones lit.}}

Every edit is a take, so **UNDO** takes it back and **SHIFT + UNDO** puts it back again. Tap **‹ Clips** to close the editor.

### The tracker

The tracker shows the clip as a list of steps from top to bottom, empty steps included, the way a tracker does. It is handy for drum clips, where you want to read every step's settings at a glance.

{{shot: arrange-notes-tracker | The tracker on a bar of drums: a nudged hat, a 1:2 hat, a FILL open hat, a ratchet, and the picked snare with a lock. The Euclid rows are beside it.}}

- **One row per step**, at the snap's size (1/16 to begin with). A step with several notes, such as a kick and a hat together, gives each note a row of its own under the step. An empty step shows dots.
- **The columns**: the step (bar.beat.step), the pad or note, velocity, length, nudge (how many ticks early or late), chance, condition, ratchet, and how many parameter locks the step has. A setting left at its default shows as a faint dot, so what you changed stands out.
- **Tap a note's row** to pick it, then change it with the Edit options. With the Draw tool, tap an empty step to put a note there: the sound of the last note you tapped, a step long. With Erase, tap a note's row to take it out.
- The step on each beat is shaded, a bar starts with a line, and a green mark shows the step playing.

With the snap off, or finer than 1/32, the tracker uses 1/16 steps, and its title line says so. The names of the parameters a step locks are coming; for now it shows how many there are.

### Timing Correct

Timing Correct pulls notes toward the beat, so a part played a little loose sits in time. Tap **TC** and it corrects the picked notes, or every note if none are picked. Its settings are the first rows of the Edit options beside the roll. Tap a row and turn the data wheel to change it:

- **Correct**: what moves. **Start** moves each note's start onto the grid. **End** moves its end. **Length** makes its length a whole number of grid steps. **Legato** holds each note until the next one starts.
- **Division**: the grid, from a whole bar to 1/64, with triplets.
- **Swing**: 50 % is straight. Higher values push every second step later, for a shuffle.
- **Window**: only notes this close to a grid line are corrected. At 100 %, every note is.
- **Strength**: how far of the way each note moves. At 100 % it lands on the line. Lower values keep some of the feel.
- **Shift**: moves the corrected notes a little early or late afterwards.

The next rows act on the picked notes straight away: **Nudge** moves them a step, **Transpose** a semitone, **Velocity** up or down, **Length** longer or shorter, **Chance** 5 % at a time, **Condition** through the trig conditions (Always, 1:2 to 8:8, then FILL, !FILL, PRE, !PRE, NEI, !NEI, 1ST and !1ST), and **Ratchet** from 1 to 8 hits. A run of turns on the same row is one take, so one **UNDO** takes it all back.

### Euclidean fill

The Euclidean fill spreads a number of hits as evenly as they go over a number of steps, a quick way to write a groove: 4 hits over 16 steps is four on the floor, 3 over 8 is the tresillo, 5 over 8 the cinquillo. It writes one sound's row, and its four rows are the last of the Edit options:

- **Euclid**: the sound it writes. It follows the last note you tapped; turn it to pick another pad or note, and that sound's notes light up.
- **Hits**: how many hits.
- **Steps**: how many steps the pattern spans (up to 64), at the snap's size, for example "16 × 1/16".
- **Rotate**: turns the pattern so it starts later.

Each turn of Hits, Steps or Rotate writes the pattern straight away, repeated through the whole clip. Steps that keep a hit keep their notes as they were; new hits are a step long and as loud as the sound's first note. Other sounds stay as they are. A pattern of 12 steps on a 16-step clip runs on into the next bar, for a part that drifts against the others. A run of turns is one take, so one **UNDO** puts the row back as it was.

> [!MPC]
> This is the MPC's Grid View, List Edit and Timing Correct in one place. TC here corrects notes already recorded. Correcting while you record is coming. The tracker and the Euclidean fill aren't on the MPC: the fill is like the Euclidean mode of Elektron's Digitakt.

> [!MPC]
> On the MPC, a sequence holds every track and you switch whole sequences. In MPFree each track has its own clips, launched on their own like Ableton Live's clips, and clips of different lengths play side by side.
