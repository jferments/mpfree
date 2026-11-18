<!--
status: changing
covers: rust/mpfree-looper rust/mpfree-app/src/looper_page.rs rust/mpfree-app/src/looper_tab.rs rust/mpfree-app/src/looping.rs rust/mpfree-seq/src/loops.rs
checked: 2026-11-16 873e19d
-->

# Recording loops

[Your first loop](../getting-started/first-loop.md) showed the basics: record a pass, overdub layers on top, undo one. This chapter covers how the looper works in more depth, the **Looper** tab, recording from an instrument, and getting the timing right.

You perform a loop on the Play page (the looper strip in the All view, or the bigger Loop view). You set it up on the **Looper** tab.

## How the looper keeps time

The looper is part of the song, not a separate machine running beside it:

- **It starts and ends on bar lines.** Press Record and the loop waits for the next bar line, then records until you press Record again, when it ends on the next bar line. Press **STOP** instead and it stops at once and keeps what you played: the loop runs on to the next bar line, silent after the stop (if you press STOP just after a bar line, within a sixteenth note, it ends on that bar line instead). With a set length (**First pass**, below) it ends by itself after that many bars. So the loop is always a whole number of bars, and always in time with your patterns.
- **It follows the song's position.** Stop and start the transport and the loop picks up exactly where the song is. It is silent while the transport is stopped, and its playhead stops too, at the place the song will start from again (press STOP twice and it goes back to the start of the loop, with the song).
- **The tempo is the song's tempo.** Change the tempo and a playing loop follows it and keeps its pitch. MPFree re-makes the loop at the new tempo in the background, which takes a fraction of a second; until it is ready, the loop plays faster or slower, like tape. Overdub over a loop at its new tempo and MPFree first fits the loop to that tempo, so the new layer lines up. The fit is its own step: UNDO takes back the overdub, then the fit.
- **There are no clicks.** The end of the loop is blended into its start, and every start, stop, punch in and out, mute and undo is faded in a few milliseconds.

## Layers

The first pass is the loop's first **layer**. Each overdub adds another, up to 16. **Undo** fades out the newest layer and **Redo** brings it back, until you record a new overdub. The loop shows the layers in different colours, and on the Play page each layer has a speaker icon to mute it on its own.

To get rid of a whole loop, first pass included, use **Clear** (on the Play page, or **Clear loop** on the Looper tab). The **×** on a loop's last remaining layer does the same. In Perform these take a half-second hold, so a stray touch can't clear a loop mid-set (see [Make and Perform](playing-live.md#make-and-perform)).

The looper holds five minutes of stereo sound across all its loops and layers (the Looper tab shows how much is left). At 120 BPM a bar lasts two seconds, so that is room for about 37 four-bar layers, spread over up to eight loops of up to 16 layers each. If you need more or less, **Loop memory** in [Prefs](../reference/prefs.md) (also in the Looper tab's MENU) sets it from 1 to 20 minutes. The memory is set aside when MPFree starts, so a new size takes effect from the next start; the row says "(next start)" until then, and shows how many megabytes it uses.

## The buttons

These are on the Play page's Loop view. The MPC's own **REC** and **OVERDUB** do the same on the loop picked while the loop is armed: the Loop view and the Looper tab arm it, and the REC chip on the transport bar says "→ Loop 1" (see [What REC records](../getting-started/first-beat.md#what-rec-records)). On the home view, the **Arm** pill at the left of the loop strip arms the loop too: red when REC records it, grey when it doesn't. **SHIFT** + Arm adds the loop to the pattern, so one REC and PLAY record the pads into a clip and a loop's first pass together, from the same bar line. **UNDO** while the first pass or an overdub records throws that take away and the loop plays on. **UNDO** (and **SHIFT + UNDO** for Redo) works on the loop on the Looper tab and the Loop view, and on the All view when the loop is what you changed last. To record a pattern from the pads with REC, open the Beats view (it arms the notes) or pick a track on the Arrange tab:

| Button | Empty loop | Recording the first pass | Playing |
|---|---|---|---|
| **Record** | Waits for the next bar line (or, stopped, for PLAY), then records. Press again before it starts to cancel. | Ends the loop on the next bar line. | Starts an overdub. |
| **Overdub** | Nothing. | Goes straight on into an overdub when the first pass ends. | Starts an overdub; press again to stop adding. |
| **Undo** | Nothing. | Throws the recording away. | Removes the newest layer (or the overdub in progress). |
| **Redo** | Nothing. | Nothing. | Brings back the last layer undone. |
| **Clear** | | | Deletes the whole loop, so you can start again. |
| **Mute** | | | Silences the loop without losing it. Press again to hear it. |

**STOP** while the first pass records stops it at once and keeps what you played, up to the next bar line, so a loop is never lost. Stopping the transport while you overdub ends the overdub. With the transport stopped, Record or Overdub on a loop that has something in it waits for **PLAY**, with OVERDUB and PLAY blinking; **STOP** then lets go of it (and of a loop armed with Record), so nothing is left blinking.

## The Looper tab

The **Looper** tab is where you set up and mix your loops. It has room for up to eight loops at once.

{{shot: looper | The Looper tab: the loops on the left with the memory they share, the picked loop's editor with a lane per layer, the recording settings below, and the Q-Links on the right.}}

### Loops

The list on the left shows each loop with its length and number of layers, and whether it is playing, muted or empty. Tap a loop to pick it: the editor shows it, and REC, OVERDUB and UNDO act on it. **+ New loop** picks the first empty one, ready to record.

- **The speaker** on each loop's card (in its box at the bottom right) decides whether the loop plays. Tap it, or anywhere in the card's bottom right corner, to mute the loop (the speaker shows a ×): the loop is kept and keeps its place in the song, but stays silent when you press PLAY. Tap it again and the loop comes back in, in time. You don't have to pick a loop to mute it.
- **Clear loop** (under + New loop, outlined in red) deletes the picked loop, every layer and the first pass, so you can record it again. It fades out rather than cutting off.

Every loop plays at once, in time with the song, but only one records at a time: starting to record in one loop ends the recording in the others.

**Loop memory** at the bottom shows how much of the looper's memory is used ("1:42 of 5 min"), shared between all the loops and their layers.

### The editor

The picked loop's waveform runs across the top, with its length, tempo and source above it. While the first pass records, the waveform grows as you play, drawn against the bars you set (with a free length, against two bars, then four, and so on). Under it is a **lane for each layer**, with its own waveform and:

- **the speaker** (in its box under the layer's name) to mute that layer,
- **S** to solo it (hear only this layer of the loop),
- **a level** you can drag,
- **×** to delete the layer. A deleted layer can be brought back with UNDO, like an overdub. On the loop's only layer (the first pass, once the others are gone), **×** clears the whole loop, like **Clear loop**.

Three lanes show at a time. When a loop has more layers, the line under the lanes says so ("swipe for more · layers 1 to 3 of 6"): **swipe up or down on the lanes** to see the others. They move a lane at a time and stay where you leave them until you pick another layer.

You can also hold **ERASE** and tap a layer's lane to delete it, with the same UNDO. ERASE leaves the loops in the list alone, since a cleared loop can't be brought back.

Tap a lane to pick that layer for the Q-Links:

| Q-Link | Turns |
|---|---|
| 1 | The picked layer's level |
| 2 | The picked layer's pan |
| 3 | **Fade old**: turns older layers down a little each time the loop goes round, from 50 % to 100 % kept per pass, so a loop can evolve as you keep overdubbing. A faded layer says how far it has gone ("faded to 77 %"). |
| 4 | The seam fade (see below) |

### Moving a layer to another loop

Drag a layer out of the editor to give it to another loop. **Press and hold** the layer's lane for a moment, until it gets a blue outline, then move your finger: a card with the layer's name follows it, the layer's own lane is outlined, and the loops in the list that can take it light up, with **+ New loop**. The line at the bottom says what will happen if you let go there.

- **Onto a loop:** the layer moves there. It keeps its level, pan and mute, and it plays in time with that loop's bars.
- **Hold SHIFT as you let go** to copy it instead: the layer stays in this loop too.
- **Onto + New loop:** the layer becomes a loop of its own, the same length as the loop it came from.
- **Let go where you started** (or anywhere that isn't lit) and nothing changes.

The layer comes in a moment later (about a sixteenth of a second for a 2-bar loop at 120 BPM), fading in where the loop has got to, and fades out of its own loop at the same moment, so you hear no gap and no doubling. Afterwards the loop that took it is picked, so the editor shows it there.

**UNDO** takes a move back in both loops at once, from either loop, as long as it was the last thing you did in each. Undoing a layer that made a loop of its own empties that loop again.

A layer only goes onto a loop of the same length in bars. A loop of another length isn't lit, and dropping on it tells you why ("Loop 2 is 2 bars and this layer 1 bar"). A loop that is recording or busy (merging, for instance) says so too, and so does a loop that already has as many layers as it can hold. A loop's last layer can't leave it, so dragging it copies it.

With the **data wheel**: pick a layer, push the wheel to pick it up, turn it to point at a loop, and push again to drop it there (hold SHIFT for a copy). Change page and it is put back.

### Recording and input

The panel at the bottom left holds how the looper records. The Play page's looper uses the same settings. Tap the right half of a row to step it up and the left half to step it down.

| Setting | What it does |
|---|---|
| **Record from** | **Pads and synth**: MPFree's own sound as you hear it, with the effects on its tracks and the echoes and reverbs of the sends. It leaves out the loops, the audio tracks, the line input and the metronome, and the master's own effects and Level, which the loop plays through anyway. **Line input**: the INPUTS on the back of the MPC. |
| **First pass** | **free** (the default): from the next bar line until you press Record again (it ends on the next bar line) or STOP (it stops at once). Or 1, 2, 4 or 8 bars. |
| **Line input** | Opens the line input. It is off every time MPFree starts. |
| **Input peak** | How loud the input has been. |
| **Monitor** | Hear the line input through the outputs, about 9 ms late. Leave it off if you already hear your instrument another way, such as through a mixer. |
| **Latency** | How late your playing reaches the looper (see below). |

Two things to know about recording the pads with their effects. The loop's own echo is recorded too if you turn up the **Looper**'s Delay send on the Play page, since that echo is part of the mix. And if you send the Loop or Audio track (or the line input) to a submix in the mixer, the looper can't leave it out of the mix, so it records the pads and the synth without their effects until that track goes back to the master.

**After recording** sets what the loop does when its first pass ends: **play**, or go straight into **overdub**. **Start** (next bar line) shows how the looper works today and can't be changed yet. **At another tempo** still reads "like tape" in this build, but loops keep their pitch, as described above.

Press **MENU** on the Looper tab for the same settings as a list, with an explanation beside each, plus **Loop level** (-24 to +6 dB) and **Loop memory**. The Loop level, and the latency once you set it by hand, are kept for you whatever project is open; the line input is off at every start, so the Looper records the pads until you turn it on.

### Editing a loop

Pick a loop, then use the tools along the top of the editor while it plays (not while it records):

- **½ length** keeps the first half; **× 2** doubles it, so you can overdub a longer phrase on top.
- **Reverse** plays the loop backwards.
- The two **nudge** buttons (**← 1/16** and **1/16 →**) move the loop a sixteenth earlier or later against the beat.
- **Seam fade** smooths the join where the loop starts again. Each tap steps through a hard cut, 10 ms, 50 ms and the longest fade.
- **Merge layers** turns all the layers into one, which frees memory for more layers. It shows its progress while it works, and UNDO takes it back like any other step.

**Trim:** while a loop can be edited, a handle sits at each end of its wave. Drag one in to cut the loop shorter; it moves in sixteenth notes, the part cut off is dimmed, and a label says how much you keep ("keep 12 of 16 beats"). Let go to trim.

### The undo history

**Undo history** opens a list of everything you did to the picked loop, oldest first: Start (the first pass), then each overdub, deleted layer, edit and merge. The step you are on is outlined, and steps you undid are faint below it. Tap any step to go back (or forward) to just after it. Drag to scroll the list. Picking another loop closes it.

{{shot: looper-history | The undo history open on a loop with two overdubs.}}

### Using a loop

**Resample**, in the Use the loop panel, records all the loops, as you hear them, into the first empty loop, starting at the next bar line. Press REC to end it. It's the way to bounce several loops down into one and start layering again.

## Loops in the Clips grid

Every loop you record also shows on the **Arrange** tab's Clips grid, on a **Loop track**: a column named **Loops A**, **Loops B** and so on, with a little wave and what it records under the name (**Pads**, for the pads and the synth). A loop's cell is named after it ("Loop 2") and shows the loop's wave and its length; while it plays, the wave lights up to where the loop is, and it turns red with **overdub** while you add a layer. The grid's title, **Loops 4 of 8 · memory 38 %**, says how many of the eight loops hold something and how much of the loop memory they use. A loop's cell launches, stops and plays in scenes like any clip, on the bar line:

- **A loop you record lands on a Loop track with nothing playing**, in the row of the scene playing (row 1 if none), and keeps playing. When every Loop track already plays a loop, MPFree adds one, so loops you record one after another still play together, as before.
- **One loop plays per Loop track**, as one clip plays per track; loops on different Loop tracks play together. Tap another loop's cell on the same Loop track and the one playing goes quiet on the next bar line as the new one comes in, both on the bar line exactly.
- **Scenes, Stop all and TRACK MUTE work on loops too.** A scene plays the loops in its row, and a Loop track with nothing in that row stops. Stop all stops every loop on the next bar line. A muted Loop track's loop goes quiet at once, or on the launch line when the mute waits (in Perform; see [Muting tracks and pads](playing-live.md#muting-tracks-and-pads)).
- **A loop that isn't playing keeps its place** in the song: launch it again and it comes back in step.
- **Clear** a loop, or undo its first pass, and its cell empties (Redo of the first pass puts it back). **½ length**, **× 2** and **Trim** change the cell's length with the loop's.
- **A Loop track past the first 8 tracks** is on the grid's next page: tap **›** over the grid, or **Show in Clips** on the Looper tab (see below).
- **The pads never play a Loop track.** Picking one keeps the pads on the track they were on; a tap on an empty Loop cell records a loop there instead (below). Drag a loop's cell to another row or another Loop track. Loops stay on Loop tracks, and clips on the other tracks.
- **Copy a loop's cell** by holding **SHIFT** as you lift it:
  - **Into another row of its own Loop track**, both rows play the same loop. A scene change from one of those rows to the other keeps the loop playing without a break.
  - **Onto an empty cell of another Loop track**, it becomes a new loop with a copy of every layer and its mix. The cell says **copying in** for a moment, then shows the wave. The copy stays quiet until you tap its cell, so you can change it on the Looper tab while the first one plays. It takes one of the eight loops and room in the loop memory, and says so when there isn't any. **UNDO** on the Looper tab, with the copy picked, takes its layers back; once they are all gone, its cell empties.

A project saved before Loop tracks opens with each of its loops on a Loop track of its own, in row 1, all playing as they did. A project saved with Loop tracks opens with nothing launched, as its clips do: tap a loop's cell, or its scene, to play it.

### Recording a loop into a cell

1. To add a Loop track, hold **SHIFT** and tap **+ Track** over the grid: it is named Loops A, Loops B and so on. **UNDO** takes it away again.
2. Tap the line under a Loop track's name to choose what its new loops record: **Pads** (the pads and the synth), **Line in** (the MPC's inputs; it turns **Line input** on) or **Resample** (the other loops, as you hear them).
3. Tap an empty cell of the Loop track. It turns red and says **Armed**, and the REC chip names it ("REC → Loops C · Drop", the row's scene name or "row 3").
4. Press **REC**: the cell says **Ready**. Press **PLAY** if the music is stopped; the loop starts on the next bar line. The cell says **REC** while it records.
5. Press **REC** again: the loop closes on the next bar line and plays in that cell. If another loop was playing on that Loop track, it stops, since one loop plays per Loop track.

With a cell armed, **OVERDUB** adds a layer to the loop that Loop track is playing. Tap the armed cell again, or arm the notes (a track's Arm pill, the Beats view), to let go of it. When all eight loops hold something, or the loop memory is nearly full, the tap and REC say so: clear a loop on the Looper tab first. A new project starts with no Loop track; the first loop you record makes Loops A, as before.

### Loops in the song

Loop tracks have lanes in the **Song view** too (tap **Song ›** above the grid), with what they record under the name. A loop's block plays like a clip's:

- **Song on** brings each loop in where its block starts and takes it out where it ends, on the tick, on any Loop track where you haven't launched a cell and that isn't muted. Launch a cell and it takes that Loop track over, as a clip does; stop it and the track goes back to the song.
- **Rec song** keeps the loops you launch on the grid, each as a block from where it came in to where it went quiet, linked to its cell.
- **Scenes → song** puts the loops in each row into the song with the clips.
- A block plays the loop where it would be if it had played all along, as a launch does: a block of a two-bar loop that starts a bar into the loop's pass starts in its second bar. A loop's block stays on its own lane: drag it along the lane, but not onto another track, and no clip goes onto a Loop track's lane.

{{shot: arrange-song-loops | The Song view with Loop tracks: Loops A and Loops B play their loops' blocks under the drums, hats and bass.}}

### A loop's name

The Play page's loop strip, the Looper tab and the REC chip name a loop by its place on the grid: **Loops B · Loop 2 · Verse** is Loop 2, on Loops B, in the row of the Verse scene ("row 2" for a row with no scene name). In the Looper tab's list, a loop on a Loop track shows its track and length ("Loops B · 2 bars"); its layers are in the editor. **Show in Clips**, in the Use the loop panel, opens the Arrange tab with the loop's cell in view.

## Recording an instrument

1. Plug the instrument into the MPC's **INPUTS** and turn **REC VOL** up until the loudest parts are near the top of the meter but never at the end of it.
2. On the Looper tab, turn **Line input** on in the Recording and input panel. **Record from** switches to Line input by itself.
3. If you can't hear the instrument any other way, turn **Monitor the input** on.
4. Press **PLAY**, then **Record**.

> [!NOTE]
> The current test build reads the INPUTS as left and right, but which jacks it hears has not been confirmed on every MPC One. If you hear nothing, check **Input peak** on the Looper tab first.

### Latency

Sound takes a few milliseconds to reach the speakers, and your playing takes a few more to come back in through the inputs. So what you play along to the loop arrives slightly late. The looper makes up for it by writing the input that much earlier in the loop, so your overdubs land where you played them.

Recording MPFree's own pads needs nothing made up: they are already in time. For the line input, MPFree estimates the latency from the audio buffer setting. To measure it exactly, connect a cable from an output to an input and set **Latency to make up for** until a recorded click lines up with the bar line.

## Coming next

- **A loop's block from its own start**: a block that starts the loop from its top wherever it sits, and **Print to audio track**, turning a loop's blocks into regions on an audio track.
- **To a pad**: send a loop to a pad as a sample on the Sample tab. You will also be able to drag a single layer onto it.
- **Save loop**: keep it on the stick with its layers. Your loops are also saved with your project, every layer with its mix (see [Saving your work](saving.md)).

> [!MPC]
> On the MPC the Looper is a separate screen with one loop, which can run free of the transport and start when the input passes a threshold. MPFree has up to eight loops with a mixable lane per layer. Its loops always follow the song's bars and tempo, can record MPFree's own pads and synth as well as the inputs, and make up for the input's latency so overdubs land in time.
