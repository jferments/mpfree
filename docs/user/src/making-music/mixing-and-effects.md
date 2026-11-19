<!--
status: changing
covers: rust/mpfree-fx/src/mixer.rs rust/mpfree-fx/src/master.rs rust/mpfree-mixer-ui rust/mpfree-app/src/mixing.rs rust/mpfree-app/src/takes.rs
checked: 2026-11-19 79fc6f63
-->

# Mixing

Every sound in MPFree, from the pads, the synth, the loop, the audio tracks and the line input, meets in one **mixer**. There you balance levels, place sounds left and right, send them to shared effects such as a delay, and group them. The effects themselves, and how to add them, are in [Effects](effects.md).

## Opening the mixer

Tap the **Mix** tab, or press **TRACK MIX**. TRACK MIX opens it on the tracks' faders, and pressed again it takes you back to the page you were on; **SHIFT + TRACK MIX** opens it straight on the pads. The Mix tab opens the mixer as you left it. The music carries on whatever page you are on.

The tab has three looks of the same strips, switched with **Faders | Effects | EQ** at the top right: **Faders**, the strips side by side with their levels, **Effects**, every strip's chain of effects at once (see [Every chain at once: Effects](#every-chain-at-once-effects)), and **EQ**, every strip's EQ curve (see [Every EQ at once: EQ](#every-eq-at-once-eq)). The tab opens on the look you used last; TRACK MIX always opens it on Faders.

{{shot: mixer | The mixer, shown here with example tracks from the preview: inserts at the top of each strip, sends A to D, pan, the fader and meter, mute and solo, and where the strip goes. The master is on the right, ending in Master safety.}}

Along the top are the mixer's five views:

| View | Shows |
|---|---|
| **Tracks** | One strip per instrument: **Drums** (the kit on the pads, and track 1's strip), **Synth**, **Loop** and **Audio** (every audio track on the Song view, summed), then a strip for each other track that has its own instrument (**Track 2**, **Track 3** ...), shown only while it has one (see [A synth for each track](synths.md#a-synth-for-each-track)). A track has one strip whatever it plays: track 1's is **Drums**, so when you give track 1 its own instrument it plays there, and the strip is called **Track 1** while it does. Kit 1's pads still play through it too, from whichever track plays them. A kit other than kit 1 plays into the strip of the first track that plays it, so it shows there too (see [A kit for each track](pads.md#a-kit-for-each-track)). |
| **Pads** | A strip for each of kit 1's pads A1 to A16, so you can set the kick's level or put an effect on just the snare, then one for each other pad that has effects of its own, named by its pad and kit ("B5 · Kit 2"; see [Effects on one pad](shaping-a-pad.md#effects-on-one-pad-or-on-the-whole-kit)). |
| **Inputs** | The line input. |
| **Submixes** | Eight submixes for grouping, then the four send returns A to D. |
| **I/O** | Where everything goes, as a list: see [The I/O view](#the-io-view). |

{{shot: mixer-instruments | The Tracks view with tracks 1 and 2 playing their own instruments (Poly and FM): track 1's plays in the first strip, called Track 1 while it does, and track 2's has its own strip after Synth, Loop and Audio.}}

**Projects saved before track 1 played through Drums** open as they sounded. If track 1 had its own instrument in such a project, or you changed anything on its own strip (its fader, an effect, a send), track 1 keeps that strip, called **Track 1 (old)**, and the log says so when the project opens. It stays that way when you save the project again. Recorded knob moves on tracks 2 and up keep moving the strip they moved before (Synth, Loop or Audio). A step lock on track 2 and up that turned an effect on one of those strips is dropped, since it can only reach the track's own effects now, and the log says how many.

The **< >** buttons page through the strips when there are more than fit.

## A channel strip

From top to bottom, each strip has:

- **Inserts**: up to eight effects that the sound goes through, in order. Tap **+ FX** to add one, or an effect's name to open it (see [Effects](effects.md#adding-an-effect-the-chain-editor)).
- **Sends A to D**: how much of the sound goes to each send return. A return holds a shared effect, so several strips can use one reverb or delay. The strip's own sound still goes on to the master, and the return adds the effect alongside it, so a delay or reverb you add to a return starts fully wet (see [Choosing an effect](effects.md#choosing-an-effect)).
- **Pan**: where the sound sits between left and right.
- **The fader** (from silent up to +6 dB) and the **meter** beside it.
- **M** (mute) and **S** (solo). Solo keeps the strip and everything it feeds or is fed by, so soloing the snare's pad still lets you hear it through the Drums track.
- **The output**: where the strip goes. A pad goes to its track; a track to the master or a submix; a submix to the master or a higher-numbered submix.

Drag a fader, pan or send with your finger, and tap M, S or the output to change them. With the mixer open, **UNDO** takes back the last change to a strip (one drag of a fader is one UNDO) and **SHIFT + UNDO** puts it back.

## Every chain at once: Effects

{{shot: mixer-effects | The Effects look: each track's effects as a row of eight slots, in the order the sound goes through them, + FX in the first free slot, and the master's row at the bottom.}}

On **Effects** each strip of the view is a row: its name and how many of its eight slots are used, then its effects from left to right, in the order the sound goes through them. The master's row is always at the bottom.

- **+ FX** sits in each strip's first free slot. Tap it to pick an effect; it goes in that slot and opens in the chain editor.
- **Tap an effect** to pick it (it is outlined): the four Q-Links and the data wheel then turn its first four knobs, as in the chain editor, while the strips stay in view. Tap it again to open it in the chain editor.
- **Drag an effect along its row** to move it earlier or later in the chain.
- A green dot means an effect is on; a blue one that it listens to another track (**SC from** says which, see [the sidechain](effects.md#listening-to-another-track-the-sidechain)); a red one with **Nothing picked** that it is set to listen and no track is picked; a grey one that it is bypassed (**Off**). Tap the **SC from** or **Nothing picked** line to open the effect with its sidechain list up.
- **Tap a strip's name** to select it, as on Faders. Picking a strip or switching back to Faders gives the Q-Links back to the selected strip's level, pan and sends.

Tracks, Pads, Inputs and Submixes work on Effects as on Faders, and **< >** page through the rows.

## Every EQ at once: EQ

{{shot: mixer-eq | The EQ look: the kick's, bass's and lead's EQs and the master's as curves, a dot for each band, where each EQ sits on the right, and + EQ on the tracks without one. The bass is the strip picked for the Q-Links.}}

On **EQ** each strip of the view is a row again: its name, its EQ drawn as a curve (a dot for each band, in the band's colour), and where the EQ sits in the chain with its cuts. It shows the strip's first EQ effect; the Channel strip's EQ stage isn't shown here.

- **+ EQ** shows on a strip without an EQ. Tap it to add one at the end of the strip's chain; its curve appears in the row.
- **Tap a curve** to pick that EQ (it is outlined), and the band under your finger if you tapped a dot: the Q-Links then turn that band's frequency, gain and Q and the EQ's output. Tap it again to open the EQ in the chain editor.
- **Drag a dot** to move its band: sideways for its frequency, up and down for its gain (a cut's dot only moves sideways). One drag is one **UNDO**.
- **Tap a strip's name** to select it and give the Q-Links back to its level, pan and sends.

## The I/O view

**I/O** shows the project's routing in one place instead of strip by strip. The master stays on the right; down the left is a list of sections, and the section you pick fills the rest.

To open it, tap the **Mix** tab and then **I/O** among the mixer's views. Press **TRACK MIX** or pick a tab to close the mixer. While the tempo follows a clock from another device, the top bar shows a tempo chip such as **120.0 from MIDI In** with a green dot; tapping it opens this view too, at **Sync** once that section is there.

{{shot: mixer-io | The I/O view's Audio section, with example strips from the preview: each strip's output, and the inputs' source and monitoring. The tempo chip at the top follows a clock on MIDI In.}}

- **Audio** has a row for every strip that has an output: the inputs, the tracks, the pad channels and the submixes, each with where it goes. An input's row also has its **Source** and **Monitor**.
- Tap a row to select that strip (the Q-Links follow it). Tap its output to open the same picker as on the strip. Tap an input's Source or Monitor to step through the choices.
- Turn the **data wheel** to move the selection up and down the list, and press **-** or **+** to step the selected row's output.
- An output of **None (silent)** shows in amber, so a strip that goes nowhere is easy to spot.

Under Audio come more sections, for the gear around the MPC (see [Playing other gear](midi-and-cv.md)):

- **MIDI** lists the ports plugged in, each with what its **Input** does and its **Thru** (the output its messages go straight on to, if any), then **Tracks out**: where each track plays (**Plays on**, the port and the channel together, or **Inside MPFree**) and the **Program** it sends.
- **Routing** shows where each port's, the pads' and each track's MIDI goes, on one grid, sends a channel of a port to a track of its own (see [Playing several tracks from your keyboards](midi-and-cv.md#playing-several-tracks-from-your-keyboards)), and sends a port straight out of another (see [Thru](midi-and-cv.md#thru-a-keyboard-straight-to-a-synth)).
- **Learn** lists every control learned with MIDI Learn: what it moves, where it stands, its range, how it takes over, and whether it is kept with the song or for every project (see [The Learn section](midi-and-cv.md#the-learn-section)).
- **Controller** makes the pads and Q-Links play your gear instead of MPFree: the port and channel, what the pads send and the CC each Q-Link sends (see [Controller mode](midi-and-cv.md#controller-mode-the-pads-and-q-links-play-your-gear)).
- **Sync** sets **Tempo from** (MPFree itself or a port's clock), what following does with start, stop and the position, the clock arriving now, and which ports send **Clock out**.
- **CV** has a row for each of the eight CV/gate outputs: its **Job**, the **Track** it follows and its setting, with the volts it gives now.
- **Monitor** shows every MIDI message in and out as it happens (see [Seeing what comes in and goes out](midi-and-cv.md#seeing-what-comes-in-and-goes-out)).

A setting for a device that isn't plugged in, or for a part of MPFree that doesn't work yet, says so on its own row, and is kept. The MIDI, Routing, Learn, Sync and CV settings are saved with the project (a port's input, ignores and delay, controls learned for every project and controller mode's settings stay with your rig instead), and so is the rest of the mixer: levels, pans, sends, mutes and every effect with its settings (see [Saving your work](saving.md#projects)). The line input's source, monitoring and arming are not part of a project, so opening one never turns the input on.

## Using the Q-Links in the mixer

Tap a strip's name to select it. The four Q-Links then turn its **Volume**, **Pan**, **Send A** and **Send B** (on Effects, until you pick an effect). Until you tap the strips, the data wheel turns the selected strip's volume too; after a tap it moves the outline from control to control, and a push on a fader, a pan or a send lets the wheel turn it (see [The wheel's outline](../getting-started/the-screen.md#the-wheels-outline)). The I/O view keeps its wheel for its list.

## Freezing and printing a strip

Tap the selected strip's name again to open its menu:

- **Freeze** records the strip's next pass through its effects and plays that recording in their place. The effects rest, and so does a track's own instrument, which frees the processor for more tracks and effects. The fader, pan, sends, mute and solo still work.
- **Thaw** brings the live effects (and instrument) back.
- **Print to a new sample** records the next pass with its effects and lets their tail ring out, then saves it as a WAV in **Samples › Prints** in the browser (for example **Synth 2026-11-14 06.12.wav**), ready to load on a pad.
- **Cancel** stops a freeze or print under way. Nothing changes.

Both record in real time while the song plays, so the strip shows **Freezing** or **Printing** with how far it has got. A freeze waits for its loop to play through once first, so the reverb and echoes from the end of the loop are already ringing at its start; freezing a 4-bar loop at 120 BPM takes 8 to 16 seconds. A print starts at the loop's next start. Stopping the song or changing the tempo during the recording stops it, and the strip says why.

**What a strip records.** A take is one pass of a loop:

- A track with its own instrument records the clip playing on that track.
- **Drums** records track 1 and the tracks that play kit 1; **Synth** and the pad strips record what the tracks that play on the pads play. Each takes the longest clip there. To freeze, every other clip playing there must fit into that one a whole number of times, so a 1-bar and a 4-bar clip freeze as 4 bars, but a 3-bar and a 4-bar clip don't freeze.
- **Loop** and **Audio** follow no clip, so they can't be frozen, only printed. A print of them, of a submix, a return, the line input or the master records the longest clip playing on any track.
- A clip that plays its steps in another order (its **Direction**, see [Clip settings](clips.md#clip-settings)) records all of its order: a reversed clip one pass, a **Ping pong** clip every pass until it comes round the same. A clip in **Random** order never repeats, so it can't be frozen, only printed (one pass).
- A take is at most 64 seconds long.

**When a frozen strip thaws by itself.** A frozen strip plays the loop it recorded. If what it recorded changes, it thaws and says so: another clip launched or stopped, a note or step edited in the clip, its Direction changed, an effect put in, taken out or switched on or off, or a different instrument on the track. A clip launched again somewhere else just moves the frozen loop with it. Muting a track with its own instrument silences its frozen strip without thawing it. Turning an effect's knobs on a frozen strip changes nothing until you thaw it, and playing the pads live isn't heard through a frozen pad strip.

> [!NOTE]
> Frozen strips aren't saved with the project. Opening a project thaws everything.

## The master and Master safety

The **Master** strip on the right is everything you hear. Its fader is the same level as the master bar on the Play page.

The last thing on the master is **Master safety**, a limiter that keeps the output from going past -0.3 dB, so a loud moment never distorts harshly or hurts your ears. It also catches the peaks a converter rebuilds between samples, which can go over the samples themselves. It shows how many times it has caught a peak. If it is catching all the time, turn something down. Its modes are in [Master safety](effects.md#master-safety).

> [!TIP]
> Distortion you want is fine: put Drive, Crush or Lo-fi on a strip and push it. Master safety only stops the final output from clipping by accident.

## Coming next

- **Automation lanes**: fader and knob moves recorded as smooth lanes. Today a move made while recording goes into the clip as a lock on its step (see [Knob moves while you record](clips.md#knob-moves-while-you-record)), and you can see and change those locks step by step on a lock lane (see [Lock lanes](sequencer.md#lock-lanes)).

> [!MPC]
> The mixer covers the MPC's routing: pad channels, tracks, submixes, four send returns and the master, with sends before or after the fader and side-chain keys. The differences: the mixer is a tab, one press of **TRACK MIX** away from any page and one more press back, solo keeps everything the strip feeds, and the master always ends in a safety limiter. Freeze is the MPC's track freeze, but it records a loop rather than the whole sequence and follows the song's tempo; Print is like the MPC's bounce to sample.
