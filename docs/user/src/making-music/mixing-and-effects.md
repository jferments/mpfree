<!--
status: changing
covers: rust/mpfree-fx/src/mixer.rs rust/mpfree-fx/src/master.rs rust/mpfree-mixer-ui rust/mpfree-app/src/mixing.rs
checked: 2026-11-14 b76c3c7
-->

# Mixing

Every sound in MPFree, from the pads, the synth, the loop, the audio tracks and the line input, meets in one **mixer**. There you balance levels, place sounds left and right, send them to shared effects such as a delay, and group them. The effects themselves, and how to add them, are in [Effects](effects.md).

## Opening the mixer

Press **TRACK MIX**, or tap the small mixer icon at the top right of the screen. The mixer opens over the page you are on, so the music and the page carry on underneath. Press TRACK MIX again to close it. **SHIFT + TRACK MIX** opens it straight on the pads.

{{shot: mixer | The mixer, shown here with example tracks from the preview: inserts at the top of each strip, sends A to D, pan, the fader and meter, mute and solo, and where the strip goes. The master is on the right, ending in Master safety.}}

Along the top are the mixer's five views:

| View | Shows |
|---|---|
| **Tracks** | One strip per instrument: **Drums** (the kit on the pads), **Synth**, **Loop** and **Audio** (every audio track on the Song view, summed), then a strip for each track that has its own instrument (**Track 1**, **Track 2** ...), shown only while it has one (see [A synth for each track](synths.md#a-synth-for-each-track)). |
| **Pads** | A strip for each of the kit's 16 pads, so you can set the kick's level or put an effect on just the snare. |
| **Inputs** | The line input. |
| **Submixes** | Eight submixes for grouping, then the four send returns A to D. |
| **I/O** | Where everything goes, as a list: see [The I/O view](#the-io-view). |

{{shot: mixer-instruments | The Tracks view with tracks 1 and 2 playing their own instruments (Poly and FM): their strips come after Drums, Synth, Loop and Audio.}}

The **< >** buttons page through the strips when there are more than fit. The chip beside them shows how much of the MPC's processor the mixer and its effects are using.

## A channel strip

From top to bottom, each strip has:

- **Inserts**: up to four effects that the sound goes through, in order. Tap **+ Insert** to add one, or an effect's name to open it (see [Effects](effects.md#adding-an-effect-the-chain-editor)).
- **Sends A to D**: how much of the sound goes to each send return. A return holds a shared effect, so several strips can use one reverb or delay.
- **Pan**: where the sound sits between left and right.
- **The fader** (from silent up to +6 dB) and the **meter** beside it.
- **M** (mute) and **S** (solo). Solo keeps the strip and everything it feeds or is fed by, so soloing the snare's pad still lets you hear it through the Drums track.
- **The output**: where the strip goes. A pad goes to its track; a track to the master or a submix; a submix to the master or a higher-numbered submix.

Drag a fader, pan or send with your finger, and tap M, S or the output to change them.

## The I/O view

**I/O** shows the project's routing in one place instead of strip by strip. The master stays on the right; down the left is a list of sections, and the section you pick fills the rest.

To open it, tap the small **I/O icon** (an arrow in over an arrow out) just left of the mixer icon in the top bar, or tap **I/O** among the mixer's views. The I/O icon lights while the view shows; tap it again, press **TRACK MIX** or pick a tab to close the mixer. While the tempo follows a clock from another device, the top bar shows a tempo chip such as **120.0 from MIDI In** with a green dot; tapping it opens this view too, at **Sync** once that section is there.

{{shot: mixer-io | The I/O view's Audio section, with example strips from the preview: each strip's output, and the inputs' source and monitoring. The tempo chip at the top follows a clock on MIDI In.}}

- **Audio** has a row for every strip that has an output: the inputs, the tracks, the pad channels and the submixes, each with where it goes. An input's row also has its **Source** and **Monitor**.
- Tap a row to select that strip (the Q-Links follow it). Tap its output to open the same picker as on the strip. Tap an input's Source or Monitor to step through the choices.
- Turn the **data wheel** to move the selection up and down the list, and press **-** or **+** to step the selected row's output.
- An output of **None (silent)** shows in amber, so a strip that goes nowhere is easy to spot.

Under Audio come three more sections, for the gear around the MPC (see [Playing other gear](midi-and-cv.md)):

- **MIDI** lists the ports plugged in, each with what its **Input** does and its **Thru**, then **Tracks out**: where each track plays (**Plays on**, the port and the channel together, or **Inside MPFree**) and the **Program** it sends.
- **Sync** sets **Tempo from** (MPFree itself or a port's clock), what following does with start, stop and the position, the clock arriving now, and which ports send **Clock out**.
- **CV** has a row for each of the eight CV/gate outputs: its **Job**, the **Track** it follows and its setting, with the volts it gives now.

A setting for a device that isn't plugged in, or for a part of MPFree that doesn't work yet, says so on its own row, and is kept. The MIDI, Sync and CV settings are saved with the project, and so is the rest of the mixer: levels, pans, sends, mutes and every effect with its settings (see [Saving your work](saving.md#projects)). The line input's source, monitoring and arming are not part of a project, so opening one never turns the input on.

## Using the Q-Links in the mixer

Tap a strip's name to select it. The four Q-Links then turn its **Volume**, **Pan**, **Send A** and **Send B**, and the data wheel turns its volume too.

## The master and Master safety

The **Master** strip on the right is everything you hear. Its fader is the same level as the master bar on the Play page.

The last thing on the master is **Master safety**, a limiter that keeps the output from going past -0.3 dB, so a loud moment never distorts harshly or hurts your ears. It shows how many times it has caught a peak. If it is catching all the time, turn something down.

> [!TIP]
> Distortion you want is fine: put Drive, Crush or Lo-fi on a strip and push it. Master safety only stops the final output from clipping by accident.

## Coming next

- **Automation**: fader and knob moves recorded into clips.

> [!MPC]
> The mixer covers the MPC's routing: pad channels, tracks, submixes, four send returns and the master, with sends before or after the fader and side-chain keys. The differences: the mixer opens over any page instead of being its own mode, solo keeps everything the strip feeds, and the master always ends in a safety limiter.
