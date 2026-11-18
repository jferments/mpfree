<!--
status: changing
covers: rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/kit.rs rust/mpfree-app/src/app.rs rust/mpfree-seq rust/mpfree-transport
checked: 2026-11-14 b76c3c7
-->

# Your first beat

In this chapter you play a drum kit, set the tempo, program a pattern one step at a time, then record one live. It takes about ten minutes.

## Play the pads

MPFree starts on the **Play** page with a drum kit on the pads. Hit a few pads: the kick is usually on pad 1, the snare on pad 2 and the hi-hats nearby. The rows on the screen are the pads, A1 at the top, each named after its sound: tap a step on a row and that pad plays there (see [The pads as rows of steps](../making-music/playing-live.md#the-pads-as-rows-of-steps)).

- **Change the kit:** hold **SHIFT** and turn the data wheel to step through every kit, or tap the chip at the top right for the list. The chip shows which kit you're on and its bank ("Kit 2 of 7 · Bank A").
- **More pads:** a kit has 128 pads, 16 to a bank. **PAD BANK B**, **C** or **D** (with **SHIFT** for E to H) put the kit's other banks on the pads, and **PAD BANK A** brings the first 16 back.
- **Louder or softer:** drag the master bar at the right of the screen, or turn the **MASTER VOL** knob on the back for the outputs and headphones.

## Set the tempo

The tempo is the speed of the music, in beats per minute (BPM), shown in the transport bar. 90 BPM suits hip-hop, 120 house, 170 drum and bass.

- **Tap it:** press **TAP TEMPO** in time with the beat you have in your head, four or five times.
- **Type it in:** press **MENU**, pick **Tempo** with the data wheel, and press **–** and **+** (or push the wheel and turn it). Press **MENU** again to close.

## Program a pattern

A **pattern** is a loop of steps, like a grid: each row is a pad, each column a moment in time. A lit square plays that pad at that moment.

1. Tap the **Sequencer** tab at the top of the screen. Its **Steps** view shows the pattern, filling most of the screen. (**Notes**, beside Steps on the right of the transport bar, shows the same clip as notes: see [Editing notes](../making-music/editing-notes.md).)
2. Tap squares to light them. A classic start: kick on steps 1, 5, 9 and 13; snare on 5 and 13; hi-hat on every odd step.
3. Press **PLAY**. The pattern plays round and round, and a line moves across the grid to show where it is. It does after you open a project too: PLAY starts the pattern on the screen when nothing else is playing.
4. Tap more squares while it plays. Changes play the next time round.
5. Press **STOP** to stop.

{{shot: sequencer-steps | The Sequencer tab's Steps: the pattern, one row per pad, with your timing under it and the picked step in the step inspector at the bottom. Its velocity, length, nudge and chance are on the Q-Links.}}

**Fine-tune a step.** Tap a lit square once to pick it: the Q-Links now show its **velocity** (how hard it plays), **length**, **nudge** (a little early or late) and **chance** (how often it plays, for variations). Press **Q-LINK** for its **condition** (play only on some passes, or only with FILL), **ratchet** (repeats within the step), a finer nudge and the ratchet's **ramp** (its repeats getting louder or softer), and again to come back. Tap the picked step again to switch it off: it keeps everything, shows as an outline with **off** in it, and another tap switches it back on. A tap on a step that is off switches it on. To take a step out for good, hold **ERASE** and tap it, or use **Clear** in the step inspector.

**The step inspector.** While a step is picked, the bottom of the Steps view (where the throws were) shows everything about it: its eight settings, with the ones the Q-Links turn outlined (the last, **Tie / Slide**, changes with a tap: **off**, **tie**, which joins the step's note to the same note before it with no new attack, or **slide**, which overlaps the note before so a synth set to glide slides into it); **Step**, **Trig** and **Sound**, which you can tap instead of pressing **Q-LINK**; its locks, each with a **×** that takes it off; **On** and **Off**, which switch the step off and back on with everything kept (the title says "off" while it is); **Copy**, then tap the step on the same row that should get a copy; and **Clear**, which takes the step out. **UNDO** brings back whatever a tap there changed. Tap **×** at its top right to close it: the step stays, the throws come back, and **Q-LINK** reaches your macros again.

**Reading the pattern.** Each square shows its step: the brighter bar is the velocity, a small label at the top left is a condition ("1:2", "FILL"), a dashed outline with a percentage is a chance, little ticks are a ratchet, a triangle at the bottom is a nudge (left for early, right for late), a bar running into the next square is a long note, and purple dots at the top right are locks. A step switched off is an outline in the row's colour with **off** in it (its locks' dots stay).

**Lock lanes.** A lock lane shows one setting step by step, under its sound, like a row of its own: the sound's tune, decay, filter, level or slice (on a sliced pad), or the track's volume, pan or sends. Every setting the pattern already locks has a lane, and **+ Lane** at the top of the pattern adds one: it lists the settings of the pad you hit last, then the track's, and the one you tap gets an empty lane, which the rows scroll to. Tap a square on a lane to lock that step at the sound's own setting, then drag it up or down (hold **SHIFT** for fine steps), or keep your finger on it and turn the data wheel. Each locked step shows a purple bar as high as its value, with the value over it. Tap a locked square again to take its lock off (the lane stays), or hold **ERASE** and tap it. The **×** in place of the lane's mute takes the lane off with all its locks; in Perform you hold it, and **UNDO** brings them back. A send's lane throws on chosen steps: lock **Delay send** high on the last two steps of a bar and only they echo. A sound's lock plays when the sound hits on that step, so a lock on a step without one waits for a hit there. With more rows than fit, swipe the rows up and down; the bar at the right shows where you are.

{{shot: sequencer-lock-lanes | Lock lanes on the Steps: Tune and Decay under the kick, a Tune lane under the snare, each locked step a purple bar with its value.}}

On a synth track the lanes go under the note rows, and **+ Lane** lists the synth's first four Q-Link settings (and any synth setting you locked on the Synth tab with steps held), then the track's. A track that plays a synth or drum machine through MIDI out (see [Playing a synth from a track](../making-music/midi-and-cv.md#playing-a-synth-from-a-track)) has **CC lanes** instead: **+ Lane** lists the controls its [device profile](../making-music/midi-and-cv.md#device-profiles) names (or, with none, the ones most gear shares, such as Mod wheel and Cutoff on CC 74), then **Bend** and **Pressure**; **Another CC** on the list's card takes any CC by number. With more lanes than fit under the notes, swipe them. Turn the data wheel on the list to reach the rows a finger can't, and push it to add the one picked.

{{shot: sequencer-gear-lanes | CC lanes on a track that plays a synth through MIDI out: the notes on top, the synth's Cutoff (CC 43) opening over the bar and a bend on step 7 underneath.}}

**Mute a row.** Each row's name is its mute button (the speaker in the box beside the name shows it): tap the name to mute that pad, so you can drop the kick out and bring it back.

**Make it longer.** A pattern starts at 16 steps, one bar of 16ths. Tap **+** at the top right of the pattern to double it to 32 steps, then 64, then 128 (eight bars). The new steps repeat the ones you have, so the beat carries on and you change what you want in the copy. **‹** and **›** turn the pages of 16 steps, and the number between them says which page you're on. **–** halves the pattern again, down to 16 steps; press **UNDO** if you want the steps it took off back.

## Program a pattern on the pads

You can also build the pattern without touching the screen, with the pads as steps.

1. Hit the pad whose steps you want, the kick say, then press **STEP SEQ**. It lights, the Sequencer tab opens on its Steps, and the 16 pads are now the kick's 16 steps: pad 1 (bottom left) is step 1, pad 16 is step 16. Pads 1, 5, 9 and 13 glow faintly to mark the beats.
2. Hit pads to put notes on those steps. A step lights in the kick's colour, brighter the harder you hit (with **FULL LEVEL** on, every step is full). While the transport is stopped you hear each one as you add it.
3. Hit a lit step and let go to take it off.
4. Press **PLAY**: a white light walks across the pads as the pattern plays, and you can keep adding and taking off steps.
5. To program another sound, hold **STEP SEQ**, hit that sound's pad (you hear it), and let go: the pads are its steps now.
6. Press **STEP SEQ** again and the pads play sounds as usual.

The chip at the top of the screen says what the pads are ("Pads: steps · Kick"); tap it to give the pads back. On the Steps view, the sound on the pads is the first row, outlined and marked "On the pads". The pads stay steps on every page until you press **STEP SEQ** again, so you can keep programming while you look at the mixer or the Synth tab.

**Longer patterns.** **PAD BANK A** to **D** show steps 1–16, 17–32, 33–48 and 49–64 on the pads, and **SHIFT** (or a second press) the four after. The lit **PAD BANK** button is the page on the pads, and the page playing blinks. **SHIFT + STEP SEQ** turns on Follow: the pads turn the page as the pattern plays. **‹** and **›** on the screen turn the pads' page too.

**Changing a step.** Hold a step pad and turn the Q-Links: the cards say "Step 5" and turn its velocity, length, nudge (how early or late it plays) and chance. Press **Q-LINK** while you still hold it for its condition (play only on some passes, or only with FILL), its ratchet (repeats within the step) and a finer nudge. Hold several step pads to change them all at once. A step you turned stays when you let go of it. On the Sequencer tab, the step inspector shows the step you hold, and its **Sound** works only while you hold it. Touch a Q-Link without turning it to read the step's value.

**Locks.** A step can play its sound differently: tuned up, shorter, darker or quieter. Hold a step pad and press **Q-LINK** until the cards say "Sound": they show the sound's tune, decay, filter and level (on the synth, its first four knobs). Turn one and the step plays with that value while the rest play the sound as it is; the card and the step pad turn purple. Hold several steps to lock them all at once. To lock a step without playing a note there (to change a sound that is still ringing), hold **SHIFT** and hit an empty step: it holds with no note, and the cards show "Sound" straight away. With a synth, you can also hold a step and turn the knobs on the Synth tab. Hold a step and press **ERASE** to take its locks off and keep its note.

**Longer notes.** Hold a step and hit a later one: the note lasts to the end of that step, and the steps in between glow dim. Hit a step inside a long note to shorten it to there.

**Copying.** Hold **COPY**, hit a step, then hit each step it should go to. To copy a whole page, hold **COPY**, press its **PAD BANK** button, then the **PAD BANK** button of the page it should go to.

**Clearing.** Hold **ERASE** and hit a step to clear it, or press a **PAD BANK** button to clear that whole page of the sound's steps. **UNDO** brings them back.

**On the synth.** With a synth on the pads, **STEP SEQ** makes the pads the steps of the whole line, every note at once, as on a one-lane step sequencer. Hold **STEP SEQ** and play a pad, or hold a chord, and let go: new steps play that (the chip says which, "Pads: steps · Poly · C3 + G3"). Hit an empty step and it plays the note or chord; hit a lit step and let go to take the whole step off. Hold a step and turn **Q-Link 1** to move its notes up or down, or play a keyboard to put the keys you hold on it. Locks, long notes, copying and clearing work as on a drum's steps. See [Steps on the synth](../making-music/synths.md#steps-on-the-synth).

**TRACK MUTE** and **16 LEVEL** take the pads over too: only one of the three has them at a time, and the lit button tells you which.

> [!MPC]
> As on the MPC, **STEP SEQ** turns the pads into step buttons lit by velocity, and the Sequencer tab shows the whole pattern on the screen. The differences: the pads stay steps on every page until you press **STEP SEQ** again, you pick the sound by holding **STEP SEQ** and hitting its pad, and **PAD BANK** turns the pages. Velocity, length, nudge and chance live on the Q-Links for the step you hold (or pick on the screen), rather than in an event list, and holding a step then hitting a later one sets the note's length, as on the MPC Live III and XL's step buttons. Locks work as on an Elektron box: a held step's knob turn sets that one step's sound, rather than recording automation.

## Record a pattern live

Programming steps is precise; playing in feels human. Try both.

1. On the Sequencer tab, press **REC**. The REC chip on the transport bar turns red and says **READY · PLAY starts**, with the clip it will record into ("→ Drums · Clip 1"). Nothing plays yet.
2. Press **PLAY**. MPFree starts playing (after a count-in, if you set one on the Arrange tab) and records what you play on the pads. Holding **REC** and pressing **PLAY** does the same in one go. Changed your mind before PLAY? **STOP** (or REC again) lets go.
3. Play a beat. Keep it simple: kick and snare first.
4. Press **STOP**. What you played appears on the pattern.
5. To add a hi-hat without erasing the rest, press **OVERDUB** instead of REC, then **PLAY**: the chip says **READY TO ADD**, and the take adds to what's there. While the music plays, **REC** punches in (replacing) and **OVERDUB** punches in adding; while it records, **REC** punches out and **OVERDUB** switches between replacing and adding.
6. Didn't like it? Press **UNDO** to take back the last recording. **SHIFT + UNDO** brings it back.

### What REC records

The REC chip on the transport bar always names what REC records: the notes of the track the pads play (into its clip), or a loop. Opening the **Sequencer** tab arms the notes, the **Looper** tab arms the loop, and picking a track on Arrange arms its notes. The Play page leaves it as it was, so there REC records whatever you armed last; the red **Arm** pill (below) says which. Nothing changes it while a take records. If you open the Looper tab while the notes are ready, the loop gets the ready (and back again). The **Rec** button on the Arrange tab's transport bar is REC too, and its **Play** and **Stop** start a ready take or let it go.

On the Play page, the loop row and the pattern each start their heading with an **Arm** pill: red on what REC records, grey on the other. Tap a grey one to record that instead, or hold **SHIFT** and tap it to record both in one pass: the pads into the clip and what you play into a loop (the chip says "Track 1 + Loop 1"; a plain tap on one pill goes back to that one alone). Tap the REC chip for the list of everything that can record: tap a row to arm it, and turn on **Pin the arm** to keep pages and track picks from moving it, so only a tap on Arm does (the chip says "REC · PINNED"). If you started recording by mistake, press **UNDO**: the take so far is thrown away and the music plays on.

#### Record several tracks at once

{{shot: play-rec-tracks-list | The REC chip's list with Drums (the pads' track) and Keys armed: a row for each track, the armed ones red, and the loop's row last. The chip says "Drums + Keys".}}

The list has a row for each track. Hold **SHIFT** and tap a track's row to record that track too: one REC and PLAY then record the pads into the pads' track and, into each other armed track, what its keyboard or drum machine plays it (set up its channel under the I/O view's [Routing](../making-music/midi-and-cv.md#playing-several-tracks-from-your-keyboards)). For example, a keyboard on channel 1 routed to **Bass** and a drum pad controller on channel 10 routed to **Perc**: arm both with SHIFT, play the pads on **Drums**, and one take records all three. Each track records into the clip it plays, or a new clip in its first empty slot (a track with no room is left out, and the hint line says so). The chip names them ("REC → Drums + Bass + Perc", or "Drums + 2 tracks" when the names don't fit). SHIFT and a tap on an added track lets it go; a plain tap on a row, or picking a track, records that one track again. One **UNDO** takes the whole take back, on every track. Up to 16 tracks record at once. For now a routed keyboard's pressure records only on the pads' track.

> [!MPC]
> As on the MPC, **REC** readies recording and **PLAY** starts it. MPFree's REC chip shows what it will record before you start.

Every recording is kept as a **take**, so UNDO and SHIFT + UNDO step back and forward through them. There's more on recording, takes and the Arrange tab in [Clips and patterns](../making-music/clips.md).

## Make it groove

With the pattern playing, turn the Q-Links on the **Play** page:

- **Swing** delays every other step a little, for a shuffled, less robotic feel. Try 55 to 60 %.
- **Tune** raises or lowers the pitch of the pad you last played.
- **Filter** and **Delay send** colour the whole kit.

{{shot: play | The All view. The Q-Links on the right control swing, tune, the drums' filter and their delay.}}

> [!TIP]
> Hold a **throw** at the bottom of the All view (Delay throw, Stutter, Tape stop and others) to play a short effect over the music while your finger is down. They're made for live moments: try a Tape stop at the end of a phrase.

Next, [Your first loop](first-loop.md) records what you just made into a loop you can play over.
