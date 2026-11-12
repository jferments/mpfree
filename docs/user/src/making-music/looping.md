<!--
status: changing
covers: rust/mpfree-looper rust/mpfree-app/src/looper_page.rs rust/mpfree-app/src/looper_tab.rs rust/mpfree-app/src/looping.rs
checked: 2026-11-12 b9b7477
-->

# Recording loops

[Your first loop](../getting-started/first-loop.md) showed the basics: record a pass, overdub layers on top, undo one. This chapter covers how the looper works in more depth, the **Looper** tab, recording from an instrument, and getting the timing right.

You perform a loop on the Play page (the looper strip in the All view, or the bigger Loop view). You set it up on the **Looper** tab.

## How the looper keeps time

The looper is part of the song, not a separate machine running beside it:

- **It starts and ends on bar lines.** Press Record and the loop waits for the next bar line; it stops after the number of bars you set. So the loop is always a whole number of bars, and always in time with your patterns.
- **It follows the song's position.** Stop and start the transport and the loop picks up exactly where the song is. It is silent while the transport is stopped.
- **The tempo is the song's tempo.** Change the tempo and a playing loop follows it and keeps its pitch. MPFree re-makes the loop at the new tempo in the background, which takes a fraction of a second; until it is ready, the loop plays faster or slower, like tape. Overdub over a loop at its new tempo and MPFree first fits the loop to that tempo, so the new layer lines up. The fit is its own step: UNDO takes back the overdub, then the fit.
- **There are no clicks.** The end of the loop is blended into its start, and every start, stop, punch in and out, mute and undo is faded in a few milliseconds.

## Layers

The first pass is the loop's first **layer**. Each overdub adds another, up to 16. **Undo** fades out the newest layer and **Redo** brings it back, until you record a new overdub. The loop shows the layers in different colours, and on the Play page each layer has a speaker icon to mute it on its own.

The looper holds a minute of stereo sound across all its layers (the Looper tab shows how much is left). At 120 BPM a bar lasts two seconds, so a 4-bar loop can have about seven layers. Keep loops short and layer them.

## The buttons

These are on the Play page's Loop view. The MPC's own **REC**, **OVERDUB** and **UNDO** buttons do the same while the Looper tab is open, on the loop picked there:

| Button | Empty loop | Recording the first pass | Playing |
|---|---|---|---|
| **Record** | Waits for the next bar line, then records. Press again before it starts to cancel. | With a free length: ends the loop on the next bar line. | Starts an overdub. |
| **Overdub** | Nothing. | Goes straight on into an overdub when the first pass ends. | Starts an overdub; press again to stop adding. |
| **Undo** | Nothing. | Throws the recording away. | Removes the newest layer (or the overdub in progress). |
| **Redo** | Nothing. | Nothing. | Brings back the last layer undone. |
| **Clear** | | | Deletes the whole loop, so you can start again. |
| **Mute** | | | Silences the loop without losing it. Press again to hear it. |

Stopping the transport while you overdub ends the overdub.

## The Looper tab

The **Looper** tab is where you set up and mix your loops. It has room for up to eight loops at once.

{{shot: looper | The Looper tab: the loops on the left with the memory they share, the picked loop's editor with a lane per layer, the recording settings below, and the Q-Links on the right.}}

### Loops

The list on the left shows each loop with its length and number of layers, and whether it is playing, muted or empty. Tap a loop to pick it: the editor shows it, and REC, OVERDUB and UNDO act on it. **+ New loop** picks the first empty one, ready to record.

Every loop plays at once, in time with the song, but only one records at a time: starting to record in one loop ends the recording in the others.

**Loop memory** at the bottom shows how much of the looper's minute of stereo sound is used, shared between all the loops and their layers.

### The editor

The picked loop's waveform runs across the top, with its length, tempo and source above it. Under it is a **lane for each layer**, with its own waveform and:

- **the speaker** to mute that layer,
- **S** to solo it (hear only this layer of the loop),
- **a level** you can drag,
- **×** to delete the layer. A deleted layer can be brought back with UNDO, like an overdub.

Tap a lane to pick that layer for the Q-Links:

| Q-Link | Turns |
|---|---|
| 1 | The picked layer's level |
| 2 | The picked layer's pan |
| 3 | The whole loop's level |
| 4 | The line input's level |

### Recording and input

The panel at the bottom left holds how the looper records. The Play page's looper uses the same settings. Tap the right half of a row to step it up and the left half to step it down.

| Setting | What it does |
|---|---|
| **Record from** | **Pads and synth**: MPFree's own sound, before the metronome and the loops themselves. **Line input**: the INPUTS on the back of the MPC. |
| **First pass** | 1, 2, 4 or 8 bars, or **free**: from the next bar line until the bar line after you press Record again. |
| **Line input** | Opens the line input. It is off every time MPFree starts. |
| **Input peak** | How loud the input has been. |
| **Monitor** | Hear the line input through the outputs, about 9 ms late. Leave it off if you already hear your instrument another way, such as through a mixer. |
| **Latency** | How late your playing reaches the looper (see below). |

**Start** (next bar line) and **After recording** (play) show how the looper works today; they can't be changed yet. **At another tempo** still reads "like tape" in this build, but loops keep their pitch, as described above.

Press **MENU** on the Looper tab for the same settings as a list, with an explanation beside each, plus **Loop level** (-24 to +6 dB).

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

The buttons along the top of the editor and the **Use the loop** panel are drawn but not working yet. They arrive in the next version:

- **Edits:** half length, double, reverse, nudge by a 1/16 either way, a softer seam at the loop point, merging layers, and the undo history.
- **Using a loop:** send it **to a pad** as a sample on the Sample tab, **to a clip** on the Arrange tab, **save** it on the stick with its layers, or **resample** it through its effects.

> [!MPC]
> On the MPC the Looper is a separate screen with one loop, which can run free of the transport and start when the input passes a threshold. MPFree has up to eight loops with a mixable lane per layer. Its loops always follow the song's bars and tempo, can record MPFree's own pads and synth as well as the inputs, and make up for the input's latency so overdubs land in time.
