<!--
status: changing
covers: rust/mpfree-seq rust/mpfree-play rust/mpfree-capture rust/mpfree-time-ui rust/mpfree-transport
checked: 2026-11-12 59ce795
-->

# Clips and patterns

A **clip** is a piece of music you recorded or programmed: a drum pattern, a bass line, a chord part. The pattern on the Play page is a clip. Clips live on **tracks**, and the **Arrange** tab shows them all as a grid so you can record new ones, keep the best takes and switch between them while the music plays.

{{shot: clips | The Arrange tab: tracks across the top, clip slots down, the record settings and the takes beside them, and the Q-Links down the right edge.}}

## Tracks and slots

The grid has a column per **track** (up to 16) and a row per **slot**. Each track plays one clip at a time, so a column might hold four different drum patterns, and you pick which one plays.

- **Tap a clip** to launch it. It starts on the next bar line, so it always lands in time. Tap a playing clip to stop it at the next bar.
- **Tap +** in an empty slot to record a new clip there.
- **Tap a track's name** to **arm** it: the pads record into that track.

A clip's border tells you what it's doing: a ring going round as it plays one pass, amber while it waits for the next bar, red while recording.

## The Q-Links on Arrange

The four Q-Link cards run down the right edge of Arrange, in the same place as on the Play page, so the knobs never move between the two. They do the same as on Play: the knobs of the Play view you picked last, as the cards show (see [The Q-Links on the Play page](playing-live.md#the-q-links-on-the-play-page)). Turn a knob, or drag up or down on its card. **Q-LINK** steps the cards to Macros 1-4 and Macros 5-8, and Play and Arrange share the bank.

Coming:
- With an audio region picked in the Song view, the cards become the audio track's level and pan, and the region's gain and fade.
- The sequencer tracks have no mixer strips of their own yet (they all play the kit), so their own level and pan aren't on the knobs.

## Recording a clip

Set up the recording in the Record panel, beside the grid:

- **Armed:** the track that records.
- **Mode:** **Replace** records over what's there; **Overdub** adds to it. The **OVERDUB** button switches this too.
- **Count-in:** a count before recording starts, so you can come in on time.
- **Loop:** plays the same bars round and round while you record, so you can keep trying a part. It counts in bars from bar 1.
- **Tempo:** the song's tempo, in BPM.

Tap a row's arrows, or the left or right of its value, to change it. Or tap the row and turn the data wheel.

Then press **REC**, play, and press **STOP**. The new notes go into the armed track's clip.

## Takes

Every recording is a **take**, listed under Takes. **UNDO** takes back the newest take, and **SHIFT + UNDO** puts it back, so you can try a part again and again without worrying about losing the last one.

## Capture: keep what you just played

You weren't recording, but you just played something great. Tap the **Capture** button under Takes (it says how many bars it would keep), or press **SHIFT + REC** in the next build, and MPFree keeps it.

MPFree is always listening to the pads. Capture keeps your **last phrase**: everything since the last bar of silence, up to 32 bars, cut to whole bars and with your exact timing.

- If you played it in time over a clip that was playing, the notes go into that clip.
- Otherwise it becomes a new clip, named "Capture 1", "Capture 2" and so on, and starts playing in time.

UNDO takes a capture back like any other take.

> [!MPC]
> Capture is like the MPC's Retro Rec, but it keeps whole bars, works whether the music was playing or stopped, and can add to the clip that's playing instead of always making a new one.

## Steps

The pattern on the Play page's **Beats** view is a clip shown as a grid of steps. Each step can have its own **velocity**, **length**, **nudge** (early or late) and **chance** (how often it plays). See [Your first beat](../getting-started/first-beat.md#program-a-pattern).

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

> [!MPC]
> On the MPC, a sequence holds every track and you switch whole sequences. In MPFree each track has its own clips, launched on their own like Ableton Live's clips, and clips of different lengths play side by side.
