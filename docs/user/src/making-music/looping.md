<!--
status: changing
covers: rust/mpfree-looper rust/mpfree-app/src/looper_page.rs rust/mpfree-app/src/looper_tab.rs
checked: 2026-11-12 b9b7477
-->

# Recording loops

The Looper tab: several loops at once, a lane per layer, recording and input settings, and sending a loop to a pad or a clip. The Play page's loop strip plays a loop; this tab is where you shape it.

{{shot: looper | The Looper tab: the loops on the left, the picked loop's layers and wave in the middle, its tools below.}}

## Editing a loop

Pick a loop, then use the tools under its layers while it plays (not while it records):

- **½ length** keeps the first half; **× 2** doubles it, so you can overdub a longer phrase on top.
- **Reverse** plays the loop backwards.
- The two **nudge** buttons move the loop a little earlier or later against the beat.
- **Seam fade** smooths the join where the loop starts again. Each tap steps through a hard cut, 10 ms, 50 ms and the longest fade.
- **Merge layers** turns all the layers into one, which frees memory for more layers. It shows its progress while it works, and Undo takes it back like any other step.

**Trim:** while a loop can be edited, a handle sits at each end of its wave. Drag one in to cut the loop shorter; it moves in sixteenth notes, the part cut off is dimmed, and a label says how much you keep ("keep 12 of 16 beats"). Let go to trim.

**Fade old** (Q-Link 3) turns older layers down a little each time the loop goes round, from 50 % to 100 % kept per pass, so a loop can evolve as you keep overdubbing. A faded layer says how far it has gone ("faded to 77 %"). **Q-Link 4** is the seam fade.

## The undo history

**Undo history** opens a list of everything you did to the picked loop, oldest first: Start (the first pass), then each overdub, deleted layer, edit and merge. The step you are on is outlined, and steps you undid are faint below it. Tap any step to go back (or forward) to just after it. Drag to scroll the list. Picking another loop closes it.

{{shot: looper-history | The undo history open on a loop with two overdubs.}}

## Recording settings

**After recording** sets what the loop does when its first pass ends: play, or go straight into overdub.

## Using a loop

**Resample** records all the loops, as you hear them, into the first empty loop, starting at the next bar line. Press REC to end it.

This chapter is not finished. Still to come:

- Loops and layers
- The line input
- Using a loop: to a pad, a clip, a file
