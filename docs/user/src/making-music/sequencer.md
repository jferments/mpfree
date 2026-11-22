<!--
status: changing
covers: rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/play_widgets.rs rust/mpfree-app/src/app/step_pads.rs rust/mpfree-app/src/step_keys.rs rust/mpfree-app/src/locks.rs rust/mpfree-app/src/rec_arm.rs rust/mpfree-app/src/rec_list.rs rust/mpfree-seq
checked: 2026-11-19 a6fb29b2
-->

# The Sequencer: programming steps

The **Sequencer** tab is where you build and fine-tune a pattern one step at a time. It shows a clip of the track you picked (the one the pads play): the clip that plays, or the one you pick with the clip chip (see [Edit another clip](#edit-another-clip)). There are two views, switched at the right end of the transport bar:

- **Steps**: the pattern as a grid, one row per pad (or per note, on a synth), with your timing under it and the picked step's settings at the bottom. This chapter is about Steps.
- **Notes**: the same clip as a piano roll, a list or a tracker, for fixing what you played in. See [Editing notes](editing-notes.md).

Press **STEP SEQ** from any page to come here, with the pads turned into the steps of a sound (see [The pads as steps](#the-pads-as-steps-step-seq)). Opening the Sequencer tab arms the track's notes, so **REC** records into this clip.

{{shot: sequencer-steps | The Sequencer tab's Steps: the kick, snare and hats with a Tune lane under the kick and the snare, your timing under the pattern, and step 7 of the kick picked in the step inspector. The Q-Links turn its velocity, length, nudge and chance.}}

## The pattern

A **pattern** is a loop of steps, like a grid: each row is a pad, each column a moment in time (a 16th note to begin with). A lit square plays that pad at that moment.

- **Tap an empty square** to put a note there. On a track with no pattern yet, the first tap makes one and starts it playing; one **UNDO** takes that pattern away again, note and all, and **SHIFT + UNDO** brings it back playing.
- **Tap a lit square** to take the note out. **UNDO** brings it back as it was, settings and all.
- **Hold your finger on a lit square** for a moment to pick it: it gets a white outline, the step inspector opens at the bottom, and the Q-Links turn its settings (below). Holding a step pad with **STEP SEQ** on does the same.
- **Hold ERASE and tap a square** also takes a step out, or use **Clear** in the step inspector.
- To drop a hit out and bring it back later without losing its settings, pick it and tap **Off** in the step inspector. It shows as an outline with **off** in it, and a tap on it switches it back on as it was.

Changes play the next time the pattern comes round. **PLAY** starts the pattern on the screen when nothing else is playing, and a line moves across the grid to show where it is.

The heading over the grid has, from the left: the red **Armed** pill, the clip chip with the pattern's name, the track's name, **+ Lane** (see [Lock lanes](#lock-lanes)), the page arrows **‹ 1/1 ›** and the length **– 16 steps +**.

### Edit another clip

The Sequencer edits the clip that plays on the track. To work on another one without starting it, tap the clip chip (the pattern's name with a ▾) and pick it from the list. What plays keeps playing, so you can build Pattern 3 while Pattern 1 plays, then launch it from Arrange when it's ready. The heading then says which clip plays ("Pattern 1 plays"), and STEP SEQ and Notes edit the clip you picked too. Each edit is a take, so UNDO takes it back.

The list starts with **What plays**, which goes back to editing the clip that plays. Turn the wheel and push to pick a row, or tap **Back** to keep the clip you edit. Picking another track also goes back to what plays.

{{shot: sequencer-edit-clips | The clip chip's list: What plays (Pattern 1, edited now), then the track's clips with their slots, Pattern 1 playing.}}

**Mute a row.** Each row's name is its mute button, with a speaker in the box beside it: tap the name to mute that pad on this track, so you can drop the kick out and bring it back. Another track playing the same kit keeps its kick, rows on every bank and kit have a mute, and a synth's rows (notes, not pads) have none. Hold **ERASE** and tap a row's name to delete all of that sound's steps.

**Make it longer.** A pattern starts at 16 steps, one bar of 16ths. Tap **+** at the top right of the pattern to double it to 32 steps, then 64, then 128 (eight bars). The new steps repeat the ones you have, so the beat carries on and you change what you want in the copy. **‹** and **›** turn the pages of 16 steps, and the number between them says which page you're on. **–** halves the pattern again, down to 16 steps; press **UNDO** if you want the steps it took off back.

With more rows than fit, swipe the rows up and down; the bar at the right shows where you are. Turning **Timing strip** off in the Sequencer's options (**MENU**) gives the pattern more rows.

### Reading the pattern

Each square shows its step:

| You see | It means |
|---|---|
| A brighter bar inside the square | The velocity: how hard it plays. |
| A label at the top left ("1:2", "FILL", "PRE") | A condition: the step plays only on some passes, or only in a fill. |
| A dashed outline with a percentage | A chance: the step plays that often. |
| Little ticks at the top right | A ratchet: the step repeats within itself. |
| A triangle at the bottom | A nudge: pointing left for early, right for late. |
| A bar running into the next square | A long note. |
| Purple dots at the top right | Locks: the step changes the sound. |
| An outline in the row's colour with **off** in it | A step switched off with the inspector's **Off** (its locks' dots stay). Tap it to switch it back on. |

## A step's settings

Hold your finger on a lit square to pick it. The Q-Links now turn its four main settings:

| Q-Link | Setting | What it does |
|---|---|---|
| 1 | **Velocity** | How hard it plays, 1 to 127. |
| 2 | **Length** | How long the note lasts, from a 1/32 to several bars. |
| 3 | **Nudge** | A little early or late, in milliseconds. |
| 4 | **Chance** | How often it plays, for variations: 100 % always, 50 % half the time. |

**Set several steps at once.** Hold **SHIFT** and tap lit squares on one row: each one is picked straight away, outlined, and a second SHIFT + tap leaves it out again. Now one turn of a Q-Link sets them all, so six hats get the same velocity with one turn, and one **UNDO** takes the whole turn back. The inspector's heading lists them ("Steps 1, 3, 5 · Hat · picked"), and its settings and locks go to every step picked. SHIFT + a tap on an empty square still puts a note there. Tapping a picked step without SHIFT takes it out, as always, and leaves the others picked. Holding a step picks it on its own, and the inspector's **×**, another page or another clip lets them all go. On the pads, holding several step pads does the same (see [The pads as steps](#the-pads-as-steps-step-seq)).

Press **Q-LINK** for the next page, **Trig**: the step's **condition** (play only on some passes, or only with a fill), its **ratchet** (2 to 8 quick repeats within the step), a finer **nudge** in ticks and the ratchet's **ramp** (its repeats getting louder or softer). Press it again for **Sound**: the sound's tune, decay, filter and level on this one step (see [Locks](#locks-a-step-that-changes-the-sound)). Press it once more to come back to Step.

The conditions are:

| Condition | The step plays |
|---|---|
| **Always** | Every time (then its chance decides). |
| **1:2** to **8:8** | On one pass out of every few: 1:2 on passes 1, 3, 5 and so on, 2:4 on passes 2, 6, 10. Passes count from the clip's start. |
| **FILL** and **!FILL** | Only while a fill is held, or only while it isn't (see [Fills](playing-live.md#fills)). |
| **PRE** and **!PRE** | Only if the row's last step with a condition (or a chance) played, or only if it didn't. Chain them for "either this or that". |
| **NEI** and **!NEI** | Only if the last such step on the row above played, or only if it didn't: a hat that answers the snare's chance. |
| **1ST** and **!1ST** | Only on the clip's first pass, or on every pass but the first. |

### The step inspector

While a step is picked, the bottom of the Steps view (where the throws are) shows everything about it:

- **Its eight settings**, with the ones the Q-Links turn outlined: velocity, length, nudge, chance, condition, ratchet, ramp and **Tie / Slide**. Tie / Slide changes with a tap: **off**; **tie**, which joins the step's note to the same note before it with no new attack; or **slide**, which overlaps the note before so a synth set to glide slides into it.
- **Step**, **Trig** and **Sound**, which you can tap instead of pressing **Q-LINK**.
- **Locks on this step**, each with a **×** that takes it off.
- **On** and **Off**, which switch the step off and back on with everything kept (the title says "off" while it is). A tap on a step that is off also switches it back on.
- **Copy**: tap it, then tap the step on the same row that should get a copy.
- **Clear**, which takes the step out.

**UNDO** brings back whatever a tap there changed. Tap **×** at its top right to close it: the step stays, the throws come back, and **Q-LINK** reaches your macros again.

## Locks: a step that changes the sound

A **lock** makes one step play its sound differently: tuned up, shorter, darker or quieter, while every other step plays the sound as it is. This is how one hi-hat sample becomes an open and closed hat, or a snare gets a pitched-up flam.

- **On the screen:** pick a step, press **Q-LINK** until the cards say **Sound**, and turn a knob. The card and the step's dots turn purple.
- **On the pads:** with **STEP SEQ** on, hold a step pad, press **Q-LINK** until the cards say "Sound", and turn one. Hold several steps to lock them all at once.
- **Without a note:** hold **SHIFT** and hit an empty step pad: it holds with no note, so you can change a sound that is still ringing from an earlier step.
- **On a synth:** hold a step and turn the knobs on the Synth tab.

Hold a step and press **ERASE** to take its locks off and keep its note.

When two tracks play the same instrument (the synth, or one track's instrument that another plays: see [Share one between tracks](synths.md#a-synth-for-each-track)) and both lock the same knob, one of them moves it at a time: the track you made first. The other track's locks wait and play once the first track's stop, the knob gliding over to them (a switch such as a waveform changes at once). Turning the knob yourself always wins: while you turn it and for half a second after, the locks wait, then the knob goes back to them.

### Lock lanes

A **lock lane** shows one setting step by step, under its sound, like a row of its own: the sound's tune, decay, filter, level or slice (on a sliced pad), or the track's volume, pan or sends. Every setting the pattern already locks has a lane, so locks you recorded live show up.

1. Hit the pad whose sound you want to lock, then tap **+ Lane** at the top of the pattern. The list shows that pad's settings first, then the track's.
2. Tap a setting (or turn the wheel to it and push). It gets an empty lane under its sound, and the rows scroll to it.
3. Tap a square on the lane to lock that step at the sound's own setting.
4. Drag it up or down (hold **SHIFT** for fine steps), or keep your finger on it and turn the data wheel. Each locked step shows a purple bar as high as its value, with the value over it.

Tap a locked square again to take its lock off (the lane stays), or hold **ERASE** and tap it. The **×** in place of the lane's mute takes the lane off with all its locks; in Perform you hold it, and **UNDO** brings them back.

A send's lane throws on chosen steps: lock **Delay send** high on the last two steps of a bar and only they echo. A sound's lock plays when the sound hits on that step, so a lock on a step without a hit waits for one there.

**On a track that plays another track's instrument** (see [Share one between tracks](synths.md#a-synth-for-each-track)), the volume, pan and sends lanes move that instrument's mixer channel, since that is where the track is heard. If the instrument's own track, or another track that plays it, locks the same setting at the same time, the track you made first moves it, and the other's lock takes over when it stops.

{{shot: sequencer-lanes-list | The list + Lane opens: the kick's settings (the pad hit last) first, then the track's volume, pan and sends.}}

{{shot: sequencer-lock-lanes | Lock lanes on the Steps: Tune and Decay under the kick, a Tune lane under the snare, each locked step a purple bar with its value.}}

**On a synth track** the lanes go under the note rows, and **+ Lane** lists the synth's first four Q-Link settings (and any synth setting you locked on the Synth tab with steps held), then the track's.

**On a track that plays outside gear** through MIDI out (see [Playing a synth from a track](midi-and-cv.md#playing-a-synth-from-a-track)) the lanes are **CC lanes**: **+ Lane** lists the controls its [device profile](midi-and-cv.md#device-profiles) names (or, with none, the ones most gear shares, such as Mod wheel and Cutoff on CC 74), then **Bend** and **Pressure**; **Another CC** on the list's card takes any CC by number. With more lanes than fit under the notes, swipe them.

{{shot: sequencer-gear-lanes | CC lanes on a track that plays a synth through MIDI out: the notes on top, the synth's Cutoff (CC 43) opening over the bar and a bend on step 7 underneath.}}

## The synth's notes as steps

With a synth on the pads, the rows are notes, the highest on top, like a piano roll turned to the scale. With a scale on, the rows are its notes, and the root's rows are shaded and marked **root**. Tap more rows on the same step for a chord. **‹ Oct ›** shows the rows an octave lower or higher, and **Fold** shows only the notes the pattern uses. See [The synth's notes on the Sequencer](synths.md#the-synths-notes-on-the-sequencer).

{{shot: sequencer-steps-synth | The Steps on a synth track in C minor: the rows are the scale's notes from C3 to C4, the roots shaded, and the eight throws underneath while no step is picked.}}

## Your timing

The **Your timing** strip under the pattern reads the last two bars you played on the pads against the 16th-note grid:

- **Average** says how early or late you play on average.
- **Spread** says how consistent you are: the smaller, the tighter.
- **Velocity** says how hard you hit, on average.
- The bars on the right show how your hits spread, from 30 ms early to 30 ms late, with the green bar on the grid.

Use it to practise finger drumming: play along with a pattern and watch the average come towards zero and the spread shrink. To give the pattern more rows, press **MENU** and set **Timing strip** to **Off**; MPFree remembers the choice.

## Throws and the Q-Links

While no step is picked, the bottom of the Steps view holds all eight **throws**, the effects you play by holding them (see [Throws](playing-live.md#throws)), and the Q-Links are the Play page's:

| | Q-Link 1 | Q-Link 2 | Q-Link 3 | Q-Link 4 |
|---|---|---|---|---|
| No step picked | Swing | Tune (last pad played) | The kit's filter | The kit's delay send |
| A step picked: Step | Velocity | Length | Nudge | Chance |
| A step picked: Trig | Condition | Ratchet | Fine nudge | Ramp |
| A step picked: Sound | Tune | Decay | Filter | Level |

## The pads as steps: STEP SEQ

You can build the pattern without touching the screen, with the pads as steps.

1. Hit the pad whose steps you want, the kick say, then press **STEP SEQ**. It lights, the Sequencer tab opens on its Steps, and the 16 pads are now the kick's 16 steps: pad 1 (bottom left) is step 1, pad 16 is step 16. Pads 1, 5, 9 and 13 glow faintly to mark the beats.
2. Hit pads to put notes on those steps. A step lights in the kick's colour, brighter the harder you hit (with **FULL LEVEL** on, every step is full). While the transport is stopped you hear each one as you add it.
3. Hit a lit step and let go to take it off.
4. Press **PLAY**: a white light walks across the pads as the pattern plays, and you can keep adding and taking off steps.
5. To program another sound, hold **STEP SEQ**, hit that sound's pad (you hear it), and let go: the pads are its steps now.
6. Press **STEP SEQ** again and the pads play sounds as usual.

The chip at the top of the screen says what the pads are ("Pads: steps · Kick"); tap it to give the pads back. On the Steps view, the sound on the pads is the first row, outlined and marked "On the pads". The pads stay steps on every page until you press **STEP SEQ** again, so you can keep programming while you look at the mixer or the Synth tab.

**Longer patterns.** **PAD BANK A** to **D** show steps 1–16, 17–32, 33–48 and 49–64 on the pads, and **SHIFT** (or a second press) the four after. The lit **PAD BANK** button is the page on the pads, and the page playing blinks. **SHIFT + STEP SEQ** turns on Follow: the pads turn the page as the pattern plays. **‹** and **›** on the screen turn the pads' page too.

**Changing a step.** Hold a step pad and turn the Q-Links: the cards say "Step 5" and turn its velocity, length, nudge and chance. Press **Q-LINK** while you still hold it for its condition, ratchet and a finer nudge, and again for its locks. Hold several step pads to change them all at once. A step you turned stays when you let go of it. The step inspector shows the step you hold, and its **Sound** works only while you hold it. Touch a Q-Link without turning it to read the step's value.

**Longer notes.** Hold a step and hit a later one: the note lasts to the end of that step, and the steps in between glow dim. Hit a step inside a long note to shorten it to there.

**Copying.** Hold **COPY**, hit a step, then hit each step it should go to. To copy a whole page, hold **COPY**, press its **PAD BANK** button, then the **PAD BANK** button of the page it should go to.

**Clearing.** Hold **ERASE** and hit a step to clear it, or press a **PAD BANK** button to clear that whole page of the sound's steps. **UNDO** brings them back.

**On the synth.** With a synth on the pads, **STEP SEQ** makes the pads the steps of the whole line, every note at once, as on a one-lane step sequencer. Hold **STEP SEQ** and play a pad, or hold a chord, and let go: new steps play that (the chip says which, "Pads: steps · Poly · C3 + G3"). Hit an empty step and it plays the note or chord; hit a lit step and let go to take the whole step off. Hold a step and turn **Q-Link 1** to move its notes up or down, or play a keyboard to put the keys you hold on it. See [Steps on the synth](synths.md#steps-on-the-synth).

**TRACK MUTE** and **16 LEVEL** take the pads over too: only one of the three has them at a time, and the lit button tells you which.

> [!MPC]
> As on the MPC, **STEP SEQ** turns the pads into step buttons lit by velocity, and the Sequencer tab shows the whole pattern on the screen. The differences: the pads stay steps on every page until you press **STEP SEQ** again, you pick the sound by holding **STEP SEQ** and hitting its pad, and **PAD BANK** turns the pages. Velocity, length, nudge and chance live on the Q-Links for the step you hold (or pick on the screen), rather than in an event list, and holding a step then hitting a later one sets the note's length, as on the MPC Live III and XL's step buttons. Locks work as on an Elektron box: a held step's knob turn sets that one step's sound, rather than recording automation.

## Recording into the pattern

Programming steps is precise; playing in feels human. To record what you play on the pads:

1. On the Sequencer tab, press **REC**. The REC chip on the transport bar turns red and says **READY · PLAY starts**, with the clip it will record into ("→ Drums · Clip 1"). Nothing plays yet.
2. Press **PLAY**. MPFree starts playing (after a count-in, if you set one on the Arrange tab's Record settings) and records what you play on the pads. Holding **REC** and pressing **PLAY** does the same in one go. Changed your mind before PLAY? **STOP** (or REC again) lets go.
3. Play. Press **STOP** when you're done: what you played appears on the pattern.
4. To add to it without erasing the rest, press **OVERDUB** instead of REC, then **PLAY**: the chip says **READY TO ADD**, and the take adds to what's there.

While the music plays, **REC** punches in (replacing) and **OVERDUB** punches in adding; while it records, **REC** punches out, and **OVERDUB** turns a replacing take into adding, or, while it adds, punches out too. OVERDUB never starts erasing in the middle of a take. Press **UNDO** to take back the last recording, and **SHIFT + UNDO** brings it back. If you started recording by mistake, **UNDO** while it records throws the take away and the music plays on.

Every recording is kept as a **take**, so UNDO and SHIFT + UNDO step back and forward through them. To have notes land on the grid as you play, set **Quantise** in the Arrange tab's Record settings. There's more on recording, takes and the Arrange tab in [Clips and patterns](clips.md).

### What REC records

The REC chip on the transport bar always names what REC records: the notes of the track the pads play (into its clip), or a loop. Opening the **Sequencer** tab arms the notes, the **Looper** tab arms the loop, and picking a track on Arrange arms its notes. The Play page leaves it as it was, so there REC records whatever you armed last; the red **Arm** pill says which. Nothing changes it while a take records. If you open the Looper tab while the notes are ready, the loop gets the ready (and back again). The **Rec** button on the Arrange tab's transport bar is REC too, and its **Play** and **Stop** start a ready take or let it go.

On the Play page, the loop row and the pattern each start their heading with an **Arm** pill: red on what REC records, grey on the other. Tap a grey one to record that instead, or hold **SHIFT** and tap it to record both in one pass: the pads into the clip and what you play into a loop (the chip says "Track 1 + Loop 1"; a plain tap on one pill goes back to that one alone).

Tap the REC chip for **Record into**, the list of everything that can record: tap a row to arm it. **Rec song** under the rows records launches, mutes and scene changes into the song as you play (see [Recording a performance into the song](arranging.md#recording-a-performance-into-the-song)). Turn on **Pin the arm** to keep pages and track picks from moving it, so only a tap on Arm does (the chip says "REC · PINNED"). While it is pinned, picking another track selects it without moving REC: the Sequencer's Steps, Notes and STEP SEQ edit that track and the pads play it, and the REC chip still names the pinned track. So with REC pinned on Chords you can pick Bass and fix its steps while the next take still goes to Chords. The pads play the track you picked, so to record them, pick the pinned track again (or tap its row in the list). The chip, or **Close**, closes the list.

{{shot: play-rec-list | The REC chip's list, Record into: the track's notes (armed) and the looper, then Rec song and Pin the arm. The right side says what moves the arm and how the REC chip reads when armed, ready and recording.}}

### Record several tracks at once

The REC chip's list has a row for each track. Hold **SHIFT** and tap a track's row to record that track too: one REC and PLAY then record the pads into the pads' track and, into each other armed track, what its keyboard or drum machine plays it (set up its channel under the I/O view's [Routing](midi-and-cv.md#playing-several-tracks-from-your-keyboards)).

For example, a keyboard on channel 1 routed to **Bass** and a drum pad controller on channel 10 routed to **Perc**: arm both with SHIFT, play the pads on **Drums**, and one take records all three. Each track records into the clip it plays, or a new clip in its first empty slot (a track with no room is left out, and the hint line says so). The chip names them ("REC → Drums + Bass + Perc", or "Drums + 2 tracks" when the names don't fit). SHIFT and a tap on an added track lets it go; a plain tap on a row, or picking a track, records that one track again. One **UNDO** takes the whole take back, on every track. Up to 16 tracks record at once. For now a routed keyboard's pressure records only on the pads' track.

{{shot: play-rec-tracks-list | The REC chip's list with Drums (the pads' track) and Keys armed: a row for each track, the armed ones red, and the loop's row last. The chip says "Drums + Keys".}}

> [!MPC]
> As on the MPC, **REC** readies recording and **PLAY** starts it. MPFree's REC chip shows what it will record before you start, and SHIFT + a track's row arms several tracks, as the MPC's multi-track record arm does.

## The Sequencer's options

Press **MENU** on the Sequencer tab for its options. **Timing strip** shows or hides Your timing; the others (the repeat rate, the tempo, the metronome and its level, the pads' level and Latch) are the same as on the Play page (see [Playing live](playing-live.md)).
