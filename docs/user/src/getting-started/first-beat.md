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
3. Press **PLAY**. The pattern plays round and round, and a line moves across the grid to show where it is. It does after you open a project too: PLAY starts the pattern on the screen when nothing else is playing.
4. Tap more squares while it plays. Changes play the next time round.
5. Press **STOP** to stop.

{{shot: play-beats | The Beats view: the pattern, one row per pad, with your timing under it. The picked step's velocity, length, nudge and chance are on the Q-Links.}}

**Fine-tune a step.** Tap a lit square once to pick it: the Q-Links now show its **velocity** (how hard it plays), **length**, **nudge** (a little early or late) and **chance** (how often it plays, for variations). Tap it again to turn it off.

**Mute a row.** Each row's name is its mute button (the speaker in the box beside the name shows it): tap the name to mute that pad, so you can drop the kick out and bring it back.

**Make it longer.** A pattern starts at 16 steps, one bar of 16ths. Tap **+** at the top right of the pattern to double it to 32 steps, then 64, then 128 (eight bars). The new steps repeat the ones you have, so the beat carries on and you change what you want in the copy. **‹** and **›** turn the pages of 16 steps, and the number between them says which page you're on. **–** halves the pattern again, down to 16 steps; press **UNDO** if you want the steps it took off back.

## Program a pattern on the pads

You can also build the pattern without touching the screen, with the pads as steps.

1. Hit the pad whose steps you want, the kick say, then press **STEP SEQ**. It lights, the Beats view opens, and the 16 pads are now the kick's 16 steps: pad 1 (bottom left) is step 1, pad 16 is step 16. Pads 1, 5, 9 and 13 glow faintly to mark the beats.
2. Hit pads to put notes on those steps. A step lights in the kick's colour, brighter the harder you hit (with **FULL LEVEL** on, every step is full). While the transport is stopped you hear each one as you add it.
3. Hit a lit step and let go to take it off.
4. Press **PLAY**: a white light walks across the pads as the pattern plays, and you can keep adding and taking off steps.
5. To program another sound, hold **STEP SEQ**, hit that sound's pad (you hear it), and let go: the pads are its steps now.
6. Press **STEP SEQ** again and the pads play sounds as usual.

The chip at the top of the screen says what the pads are ("Pads: steps · Kick"); tap it to give the pads back. On the Beats view, the sound on the pads is the first row, outlined and marked "On the pads". The pads stay steps on every page until you press **STEP SEQ** again, so you can keep programming while you look at the mixer or the Synth tab.

**Longer patterns.** **PAD BANK A** to **D** show steps 1–16, 17–32, 33–48 and 49–64 on the pads, and **SHIFT** (or a second press) the four after. The lit **PAD BANK** button is the page on the pads, and the page playing blinks. **SHIFT + STEP SEQ** turns on Follow: the pads turn the page as the pattern plays. **‹** and **›** on the screen turn the pads' page too.

**Changing a step.** Hold a step pad and turn the Q-Links: the cards say "Step 5" and turn its velocity, length, nudge (how early or late it plays) and chance. Press **Q-LINK** while you still hold it for its condition (play only on some passes, or only with FILL), its ratchet (repeats within the step) and a finer nudge. Hold several step pads to change them all at once. A step you turned stays when you let go of it. Touch a Q-Link without turning it to read the step's value.

**Longer notes.** Hold a step and hit a later one: the note lasts to the end of that step, and the steps in between glow dim. Hit a step inside a long note to shorten it to there.

**Copying.** Hold **COPY**, hit a step, then hit each step it should go to. To copy a whole page, hold **COPY**, press its **PAD BANK** button, then the **PAD BANK** button of the page it should go to.

**Clearing.** Hold **ERASE** and hit a step to clear it, or press a **PAD BANK** button to clear that whole page of the sound's steps. **UNDO** brings them back.

**TRACK MUTE** and **16 LEVEL** take the pads over too: only one of the three has them at a time, and the lit button tells you which.

> [!MPC]
> As on the MPC, **STEP SEQ** turns the pads into step buttons lit by velocity, and the Beats view shows the whole pattern on the screen. The differences: the pads stay steps on every page until you press **STEP SEQ** again, you pick the sound by holding **STEP SEQ** and hitting its pad, and **PAD BANK** turns the pages. Velocity, length, nudge and chance live on the Q-Links for the step you hold (or pick on the screen), rather than in an event list, and holding a step then hitting a later one sets the note's length, as on the MPC Live III and XL's step buttons.

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
