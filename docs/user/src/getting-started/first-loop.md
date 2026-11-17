<!--
status: changing
covers: rust/mpfree-looper rust/mpfree-app/src/looping.rs rust/mpfree-app/src/looper_page.rs rust/mpfree-app/src/play_page.rs
checked: 2026-11-16 873e19d
-->

# Your first loop

A **looper** records a few bars of what you play and plays them back round and round, in time. Then you play over the top and record that too, layer on layer, until you have a whole track. This is how many live performers build a song from nothing.

In this chapter you record a drum loop from the pads, add layers to it, and undo one.

## Open the Loop view

On the **Play** page, tap **Loop** in the view switch. The loop takes the top of the screen, with its buttons under it: **Record**, **Overdub**, **Stop**, **Undo**, **Redo**, **Mute** and **Clear**.

{{shot: play-loop | The Loop view with a loop of three layers playing: Drums, Bass and Chop. Each layer's name and speaker are one button that mutes it.}}

## Record the first pass

1. Set the tempo you want (see [Your first beat](first-beat.md#set-the-tempo)). The loop keeps it.
2. Press **PLAY**. The transport bar counts bars and beats.
3. Tap **Record**. The loop waits for the start of the next bar, then records. You'll see **RECORDING**, and the waveform of what you play grows across the loop as it records.
4. Play the pads, a beat as long as you like. The loop keeps recording until you end it.
5. Tap **Record** again just before the end of your beat. The loop closes at the end of that bar and starts playing back what you played. You'll see **PLAYING**.

As on the MPC, you can also press **REC** with the transport stopped, then **PLAY**. REC records the loop when the loop is armed: the Loop view and the Looper tab arm it, and the REC chip on the transport bar then says "→ Loop 1" (see [What REC records](first-beat.md#what-rec-records)). With the transport stopped, REC then PLAY records the loop from the first bar until you press **REC** again or **STOP**. **STOP** stops at once and keeps what you played; the loop runs on to the end of that bar, silent after the stop, and **PLAY** plays it.

The loop records MPFree's own sound as you hear it: the pads and the synth with their effects, such as the Play page's delay. It leaves out the loops already playing and the metronome. Because it always starts and ends on a bar line, it stays in time with everything else.

## Add layers

1. Tap **Overdub** and play something new over the loop: a hi-hat pattern, a bass line on the synth, a vocal chop.
2. Tap **Overdub** again to stop adding. Each overdub becomes a **layer**, named after what you played into it (Drums, Bass, Chop).
3. Repeat as often as you like.

**Undo** fades out the newest layer; **Redo** brings it back. Tapping a layer's name (or the speaker beside it) mutes just that layer, so you can drop the drums out for a breakdown and bring them back.

**Mute** silences the whole loop without losing it. **Clear** deletes the loop so you can start again.

## Record from an instrument

The looper can record the **INPUTS** on the back of the MPC instead: a synth, a guitar pedal, a turntable.

1. Plug your instrument into the INPUTS and set **REC VOL** so it's loud but not distorting.
2. Press **SHIFT + MENU** to open Prefs, pick **Line input** and switch it **On**. Press **MENU** to close Prefs.
3. The loop's heading now says it records from the line input. Record as before.

The **Looper** tab has the full set of loop controls: several loops at once, each layer's level and pan, the recording settings and the input. See [Recording loops](../making-music/looping.md).

> [!MPC]
> On the MPC the Looper is a separate screen with one loop, which can run free of the transport (Sync off) and start when the input passes a threshold. In MPFree loops always follow the song's tempo and bar lines, sit on the Play page next to your pattern, and can record MPFree's own pads and synth as well as the inputs. Up to eight loops can play at once.

> [!TIP]
> Line input is off every time MPFree starts, so you never hear a noisy input you forgot was plugged in.
