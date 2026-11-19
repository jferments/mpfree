<!--
status: changing
covers: rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/kit.rs rust/mpfree-app/src/app.rs rust/mpfree-seq rust/mpfree-transport
checked: 2026-11-19 a6fb29b2
-->

# Your first beat

In this chapter you play a drum kit, set the tempo, program a pattern one step at a time, shape it, then record a part live. It takes about ten minutes. Each step links to the chapter that covers it in full.

## Play the pads

MPFree starts on the **Play** page with a drum kit on the pads. Hit a few pads: the kick is usually on pad 1, the snare on pad 2 and the hi-hats nearby. Hit harder for louder.

The rows in the middle of the Play page are the pads of bank A, **A1** at the top, each named after its sound. **PADS PLAY** at the right of the transport bar says what the pads play, such as **Track 1 · Classic House · A**.

- **Change the kit:** tap **PADS PLAY** for the list of kits and tap one, or hold **SHIFT** and turn the data wheel to step through them.
- **More pads:** a kit has 128 pads, 16 to a bank. **PAD BANK B**, **C** or **D** (with **SHIFT** for E to H) put the kit's other banks on the pads, and **PAD BANK A** brings the first 16 back.
- **Louder or softer:** drag the master bar at the right of the screen, or turn the **MASTER VOL** knob on the back for the outputs and headphones.

## Set the tempo

The tempo is the speed of the music, in beats per minute (BPM), shown in the transport bar. 90 BPM suits hip-hop, 120 house, 170 drum and bass.

- **Tap it:** press **TAP TEMPO** (or tap the BPM on the transport bar) in time with the beat you have in your head, four or five times.
- **Type it:** hold a finger on the BPM and type the tempo on the number pad.
- **Turn it:** press **MENU**, pick **Tempo** with the data wheel, and press **–** and **+** (or push the wheel and turn it). Press **MENU** again to close.

## Program a pattern

A **pattern** is a loop of steps, like a grid: each row is a pad, each column a moment in time. A lit square plays that pad at that moment.

You can tap steps in right on the Play page's rows, but the **Sequencer** tab shows more of the pattern, so use that:

1. Tap the **Sequencer** tab at the top of the screen. Its **Steps** view shows the pattern.
2. Tap squares to light them. A classic start: kick on steps 1, 5, 9 and 13; snare on 5 and 13; hi-hat on every odd step.
3. Press **PLAY**. The pattern plays round and round, and a line moves across the grid to show where it is.
4. Tap more squares while it plays. Changes play the next time round.
5. Press **STOP** to stop. Press it twice to silence anything still ringing.

{{shot: sequencer-steps | The Sequencer tab's Steps: the pattern, one row per pad, with your timing under it and the picked step in the step inspector at the bottom.}}

**Change a step.** Tap a lit square once to pick it: the Q-Links now turn its **velocity** (how hard it plays), **length**, **nudge** (a little early or late) and **chance** (how often it plays). Tap the picked square again to switch it off without losing its settings; tap once more to bring it back. Hold **ERASE** and tap a square to take it out.

**Make it longer.** Tap **+** at the top right of the pattern to double it to 32 steps (two bars). The new bar repeats the first, so change what you like in the copy.

The [Sequencer chapter](../making-music/sequencer.md) has the rest: conditions, ratchets, locks that change the sound on one step, and lock lanes.

## Program it on the pads

You can build a pattern without looking at the screen too:

1. Hit the kick's pad, then press **STEP SEQ**. The 16 pads are now the kick's 16 steps, pad 1 (bottom left) is step 1, and pads 1, 5, 9 and 13 glow faintly to mark the beats.
2. Hit pads to put kicks on those steps, and hit a lit one to take it off.
3. Hold **STEP SEQ**, hit the snare's pad and let go: the pads are the snare's steps now.
4. Press **STEP SEQ** again and the pads play sounds as usual.

See [The pads as steps](../making-music/sequencer.md#the-pads-as-steps-step-seq) for longer patterns, long notes, copying and locks on the pads.

## Record a part live

Programming steps is precise; playing in feels human. Try both.

1. On the Sequencer tab, press **REC**. The REC chip on the transport bar turns red and says **READY · PLAY starts**.
2. Press **PLAY**. MPFree starts playing and records what you play on the pads.
3. Play a part: a shaker, a percussion line, some ghost notes on the snare.
4. Press **STOP**. What you played appears on the pattern.
5. Didn't like it? Press **UNDO** to take it back. **SHIFT + UNDO** brings it back.

To add to what's there without erasing anything, press **OVERDUB** instead of REC. See [Recording into the pattern](../making-music/sequencer.md#recording-into-the-pattern).

## Make it groove

With the pattern playing, go back to the **Play** page and turn the Q-Links:

- **Swing** (Q-Link 1) delays every other step a little, for a shuffled, less robotic feel. Try 55 to 60 %.
- **Tune** (Q-Link 2) raises or lowers the pitch of the pad you last played.
- **Filter** and **Delay send** (Q-Links 3 and 4) colour the whole kit.

{{shot: play | The Play page: the loops along the top, the pads as rows of steps, the throws and the XY pad, with swing, tune, filter and delay on the Q-Links.}}

> [!TIP]
> Hold a **throw** at the bottom of the Play page (Delay throw, Stutter, Tape stop and others) to play a short effect over the music while your finger is down. They're made for live moments: try a Tape stop at the end of a phrase. See [Throws](../making-music/playing-live.md#throws).

Next, [Your first loop](first-loop.md) records what you just made into a loop you can play over.
