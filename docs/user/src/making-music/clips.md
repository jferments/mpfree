<!--
status: changing
covers: rust/mpfree-seq rust/mpfree-play rust/mpfree-capture rust/mpfree-time-ui/src/view.rs rust/mpfree-time-ui/src/widgets.rs rust/mpfree-transport
checked: 2026-11-14 b76c3c7
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
- **Under each name** is what the track plays: **Kit**, or its own instrument (FM, Poly, a plugin synth). Tap that line to change it: the track is armed, and the Synth tab opens its MENU on the track's instrument row (see [A synth for each track](synths.md#a-synth-for-each-track)).

{{shot: arrange-instruments | Each track's instrument under its name: Track 2 plays FM, Track 4 Poly and Track 6 a plugin synth, and the rest play the kit.}}

A clip's border tells you what it's doing: a ring going round as it plays one pass, amber while it waits for the next bar, red while recording.

## The Q-Links on Arrange

The four Q-Link cards run down the right edge of Arrange, in the same place as on the Play page, so the knobs never move between the two. They do the same as on Play: the knobs of the Play view you picked last, as the cards show (see [The Q-Links on the Play page](playing-live.md#the-q-links-on-the-play-page)). Turn a knob, or drag up or down on its card. **Q-LINK** steps the cards to Macros 1-4 and Macros 5-8, and Play and Arrange share the bank.

With an audio region picked in the Song view, the cards turn the audio track's level and pan and the region's gain and fades instead (see [Recording audio tracks](audio-tracks.md#editing-regions)).

Coming:
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
- The note editor shows it: a note you leaned into has a lighter shape along its bottom, as high as you pressed (see [Editing notes](editing-notes.md)).
- Capture keeps it too, and the song plays it wherever a clip is placed.
- Turn **Pressure** to **Off** for a track that shouldn't record it, for example drums you want to stay steady. Capture leaves it out on that track too.
- Coming: editing pressure in the note editor.

## Takes

Every recording is a **take**, listed under Takes. **UNDO** takes back the newest take, and **SHIFT + UNDO** puts it back, so you can try a part again and again without worrying about losing the last one.

## Capture: keep what you just played

You weren't recording, but you just played something great. Tap the **Capture** button under Takes (it says how many bars it would keep), or press **SHIFT + REC**, and MPFree keeps it.

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

To fix what you played (pull notes into time, move them, change how hard they hit, draw new ones), tap **Edit ›** above the clip grid. The note editor has a chapter of its own: [Editing notes](editing-notes.md).

> [!MPC]
> On the MPC, a sequence holds every track and you switch whole sequences. In MPFree each track has its own clips, launched on their own like Ableton Live's clips, and clips of different lengths play side by side.
