<!--
status: changing
covers: rust/mpfree-seq rust/mpfree-play rust/mpfree-capture rust/mpfree-time-ui rust/mpfree-transport
checked: 2026-11-11 414faa1
-->

# Clips and patterns

A **clip** is a piece of music you recorded or programmed: a drum pattern, a bass line, a chord part. The pattern on the Play page is a clip. Clips live on **tracks**, and the **Arrange** tab shows them all as a grid so you can record new ones, keep the best takes and switch between them while the music plays.

{{shot: clips | The Arrange tab: tracks across the top, clip slots down, the record settings and the takes on the right.}}

## Tracks and slots

The grid has a column per **track** (up to 16) and a row per **slot**. Each track plays one clip at a time, so a column might hold four different drum patterns, and you pick which one plays.

- **Tap a clip** to launch it. It starts on the next bar line, so it always lands in time. Tap a playing clip to stop it at the next bar.
- **Tap +** in an empty slot to record a new clip there.
- **Tap a track's name** to **arm** it: the pads record into that track.

A clip's border tells you what it's doing: a ring going round as it plays one pass, amber while it waits for the next bar, red while recording.

## Recording a clip

Set up the recording on the right of the Arrange tab:

- **Armed track:** the track that records.
- **Record mode:** **Replace** records over what's there; **Overdub** adds to it. The **OVERDUB** button switches this too.
- **Count-in:** a count before recording starts, so you can come in on time.
- **Loop, bars:** plays the same bars round and round while you record, so you can keep trying a part.
- **Tempo:** the song's tempo.

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

> [!MPC]
> On the MPC, a sequence holds every track and you switch whole sequences. In MPFree each track has its own clips, launched on their own like Ableton Live's clips, and clips of different lengths play side by side.
