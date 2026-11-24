<!--
status: changing
covers: rust/mpfree-looper rust/mpfree-app/src/looping.rs rust/mpfree-app/src/looper_page.rs rust/mpfree-app/src/loop_row.rs rust/mpfree-app/src/play_page.rs
checked: 2026-11-24 459299e3
-->

# Your first loop

A **looper** records a few bars of what you play and plays them back round and round, in time. Then you play over the top and record that too, layer on layer, until you have a whole track. This is how many live performers build a song from nothing.

In this chapter you record a drum loop from the pads, add layers to it, and undo one.

## The loops on the Play page

The **Play** page has a row of all eight loops under the transport bar, a card each: the loop's name, its **ring**, and once it has sound, its speaker and **S** (solo) beside the ring. An empty loop shows a dashed ring with **+** and says what it records from (**Pads**). The row's heading names the picked loop's layers, and has **Capture** and **Pads: loop keys** at the right.

{{shot: play-loop | Play's loop row with Loop 1 playing three layers: Drums, Bass and Chop, the Chop muted from its chip. The loop is armed, so REC records it.}}

## Record the first pass

1. Set the tempo you want (see [Your first beat](first-beat.md#set-the-tempo)). The loop keeps it.
2. Press **PLAY**. The transport bar counts bars and beats.
3. Tap **Loop 1's ring**. The ring turns red and waits for the next bar line, counting down the beats in the last bar ("3 beats to REC"; a tap in the bar's last beat waits for the bar after, so you always get at least a beat to get ready). Then it records, going round with the bars.
4. Play the pads. With a pattern playing, the loop is as long as the pattern (**Length: Pattern**, the default) and closes by itself, so it always fits the beat. With nothing else playing, play a beat as long as you like: the loop keeps recording until you end it.
5. With nothing else playing, tap the ring again just before the end of your beat. The loop closes at the end of that bar and starts playing back what you played, the ring in the loop's colour.

As on the MPC, you can also press **REC** with the transport stopped, then **PLAY**. REC records the loop when the loop is armed: tap **Arm** at the left of the loop row (or open the Looper tab), and the REC chip on the transport bar then says "→ Loop 1" (see [What REC records](../making-music/sequencer.md#what-rec-records)). With the transport stopped, REC then PLAY records the loop from the first bar until you press **REC** again or **STOP**. **STOP** stops at once and keeps what you played; the loop runs on to the end of that bar, silent after the stop, and **PLAY** plays it.

The loop records MPFree's own sound as you hear it: the pads and the synth with their effects, such as the Play page's delay. It leaves out the loops already playing and the metronome. Because it always starts and ends on a bar line, it stays in time with everything else.

## Add layers

1. Tap the playing loop's ring (or press **OVERDUB**) and play something new over the loop: a hi-hat pattern, a bass line on the synth, a vocal chop.
2. Tap the ring again to stop adding. Each overdub becomes a **layer**, named after what you played into it (Drums, Bass, Chop).
3. Repeat as often as you like.

**UNDO** fades out the newest layer; **SHIFT + UNDO** brings it back. The layers of the picked loop are chips in the row's heading: tap one to mute just that layer, so you can drop the drums out for a breakdown and bring them back.

The speaker beside a ring silences the whole loop without losing it, and **S** plays it alone. Hold a ring for a second to clear the loop so you can start again; if you didn't mean it, **UNDO** brings it back.

## Record from an instrument

The looper can record the **INPUTS** on the back of the MPC instead: a synth, a guitar pedal, a turntable.

1. Plug your instrument into the INPUTS and set **REC VOL** so it's loud but not distorting.
2. Turn the line input on: press **SHIFT + MENU** to open Prefs, pick **Line input** and switch it **On**, then press **MENU** to close Prefs. (The Looper tab's options, **MENU** there, have the same switch.)
3. The loop's heading now says it records from the line input. Record as before.

The **Looper** tab (**SHIFT + SAMPLER**) is where a loop is shaped: the same eight rings across the top, and the picked loop laid out underneath with a lane per layer, its level and pan, and its recording settings. Press **SHIFT + SAMPLER** again there and the pads play the loops instead of the kit. See [Recording loops](../making-music/looping.md).

> [!MPC]
> On the MPC the Looper is a separate screen with one loop, its length set in bars before you record. With Sync off it records and plays while the sequencer is stopped, and only then can it start when the input passes a threshold. In MPFree loops follow the song's tempo and bar lines, or, with **Sync: Free**, keep their own time and play on with the song stopped, as with Sync off ([Free loops](../making-music/looping.md#free-loops)). They sit on the Play page next to your pattern, and can record MPFree's own pads and synth as well as the inputs. A loop can also wait for your first note, or for the first sound on the line input. Up to eight loops can play at once.

> [!TIP]
> Line input is off every time MPFree starts, so you never hear a noisy input you forgot was plugged in.
