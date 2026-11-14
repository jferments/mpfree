<!--
status: changing
covers: rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/kit.rs rust/mpfree-app/src/app.rs rust/mpfree-seq rust/mpfree-transport
checked: 2026-11-14 b76c3c7
-->

# Your first beat

In this chapter you play a drum kit, set the tempo, program a pattern one step at a time, then record one live. It takes about ten minutes.

## Play the pads

MPFree starts on the **Play** page with a drum kit on the pads. Hit a few pads: the kick is usually on pad 1, the snare on pad 2 and the hi-hats nearby. The pattern on the screen has a row for each pad, named after its sound.

- **Change the kit:** press **PAD BANK B**, **C** or **D** (with **SHIFT** for E to H), or hold **SHIFT** and turn the data wheel to step through every kit. Press **PAD BANK A** to come back. The chip at the top right shows which kit you're on ("Kit 2 of 7 · Bank A").
- **Louder or softer:** drag the master bar at the right of the screen, or turn the **MASTER VOL** knob on the back for the outputs and headphones.

## Set the tempo

The tempo is the speed of the music, in beats per minute (BPM), shown in the transport bar. 90 BPM suits hip-hop, 120 house, 170 drum and bass.

- **Tap it:** press **TAP TEMPO** in time with the beat you have in your head, four or five times.
- **Type it in:** press **MENU**, pick **Tempo** with the data wheel, and press **–** and **+** (or push the wheel and turn it). Press **MENU** again to close.

## Program a pattern

A **pattern** is a loop of steps, like a grid: each row is a pad, each column a moment in time. A lit square plays that pad at that moment.

1. Tap **Beats** in the view switch on the right of the transport bar. The pattern fills most of the screen.
2. Tap squares to light them. A classic start: kick on steps 1, 5, 9 and 13; snare on 5 and 13; hi-hat on every odd step.
3. Press **PLAY**. The pattern plays round and round, and a line moves across the grid to show where it is.
4. Tap more squares while it plays. Changes play the next time round.
5. Press **STOP** to stop.

{{shot: play-beats | The Beats view: the pattern, one row per pad, with your timing under it. The picked step's velocity, length, nudge and chance are on the Q-Links.}}

**Fine-tune a step.** Tap a lit square once to pick it: the Q-Links now show its **velocity** (how hard it plays), **length**, **nudge** (a little early or late) and **chance** (how often it plays, for variations). Tap it again to turn it off.

**Mute a row.** Each row's name is its mute button (the speaker in the box beside the name shows it): tap the name to mute that pad, so you can drop the kick out and bring it back.

**Make it longer.** A pattern starts at 16 steps, one bar of 16ths. Tap **+** at the top right of the pattern to double it to 32 steps, then 64, then 128 (eight bars). The new steps repeat the ones you have, so the beat carries on and you change what you want in the copy. **‹** and **›** turn the pages of 16 steps, and the number between them says which page you're on. **–** halves the pattern again, down to 16 steps; press **UNDO** if you want the steps it took off back.

> [!MPC]
> On the MPC, step sequencing has its own mode (**STEP SEQ**) where the pads become step buttons. In MPFree the pattern is on the screen, and **STEP SEQ** takes you to this Beats view. Velocity, length, nudge and chance live on the Q-Links for the picked step, rather than in an event list.

## Record a pattern live

Programming steps is precise; playing in feels human. Try both.

1. On the Beats view, press **REC**. MPFree starts playing and records what you play on the pads. (On the All and Loop views, REC records a loop instead; see [Your first loop](first-loop.md).)
2. Play a beat. Keep it simple: kick and snare first.
3. Press **STOP**. What you played appears on the pattern.
4. To add a hi-hat without erasing the rest, press **OVERDUB** first to switch overdub on, then **REC**, play the hi-hat, and press **STOP**. With overdub off, a new recording replaces what was there; with it on, it adds. Press **OVERDUB** again to switch it off.
5. Didn't like it? Press **UNDO** to take back the last recording. **SHIFT + UNDO** brings it back.

> [!MPC]
> On the MPC, **REC** arms recording and **PLAY** starts it. In MPFree, **REC** starts recording straight away (after a count-in, if you set one on the Arrange tab), and **OVERDUB** is a switch that decides whether the next recording adds or replaces.

Every recording is kept as a **take**, so UNDO and SHIFT + UNDO step back and forward through them. There's more on recording, takes and the Arrange tab in [Clips and patterns](../making-music/clips.md).

## Make it groove

With the pattern playing, turn the Q-Links on the **All** view (tap **All** in the view switch):

- **Swing** delays every other step a little, for a shuffled, less robotic feel. Try 55 to 60 %.
- **Tune** raises or lowers the pitch of the pad you last played.
- **Filter** and **Delay send** colour the whole kit.

{{shot: play | The All view. The Q-Links on the right control swing, tune, the drums' filter and their delay.}}

> [!TIP]
> Hold a **throw** at the bottom of the All view (Delay throw, Stutter, Tape stop and others) to play a short effect over the music while your finger is down. They're made for live moments: try a Tape stop at the end of a phrase.

Next, [Your first loop](first-loop.md) records what you just made into a loop you can play over.
