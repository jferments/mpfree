<!--
status: changing
covers: rust/mpfree-export rust/mpfree-browser-ui/src/export.rs rust/mpfree-app/src/export.rs
checked: 2026-11-14 97a6b00
-->

# Exporting your music

**Export** turns your song into files you can take anywhere: a finished stereo mix to share, each track on its own (stems) to mix in a computer, and a MIDI file of the notes. One export makes all three in one go, lined up to the sample, in a new folder on the stick.

## Making an export

1. Press **BROWSE**, pick **Projects** on the left and press **Export**. The Export panel opens over the browser.
2. Tick what to make (all three are ticked the first time; after that the panel remembers your last choice until MPFree restarts):
   - **Mixdown**: the whole song in one stereo file, as the speakers play it.
   - **Stems**: one stereo file for each part of the mixer that plays (Drums, Synth, Loop, Audio, each track's own instrument, and each return), after its fader, pan and mute.
   - **MIDI file**: the notes and controls each track played.
3. Pick the **Range**:
   - **Arrangement**: the song on Arrange, from its start to its end. It is picked for you when the song has one.
   - **Bars**: the clips playing now, from the start of the bar the playhead is in, for the number of bars you set (8 to start with). Turn the data wheel, or tap **–** and **+** on the panel, to change it.
4. Press **Start**, or push the data wheel.

While it runs, the panel shows how far it has got ("1:12 of 3:50"). **Cancel** stops it. When it is done, the panel says what it made, and **Show in BROWSE** opens the folder.

You can close the browser (**BROWSE**) while an export runs and go on with something else: the export keeps going, and BROWSE brings the panel back.

The export starts the song from the beginning of the range and plays it once. When it is over, the transport goes back to where it was, with the loop and Song mode you had. Pressing **STOP** before the end cancels the export.

What you play on the pads while an export runs goes into it, so leave them alone unless you mean it.

## Fast or Listen

An export plays the song in one of two ways. **Prefs > Export** picks which:

- **Fast, quiet** (the default): the song plays as fast as the MPC can make it, with the speakers quiet. How much faster than real time depends on how hard the song works the MPC: a light song is done in a fraction of its length, and one that already works the MPC hard takes about as long as it plays.
- **Listen**: the song plays once in real time, and you hear it as it records.

Some songs **always listen**, because part of their sound only exists in real time. The panel says which reason applies before you start:

- a track plays outside gear over MIDI;
- the line input is on;
- the CV or gate jacks are in use;
- MIDI clock goes out to other gear;
- the tempo follows MIDI clock from other gear;
- the song uses plugins (for now).

## What you get

The files go in a new folder in **Recordings**, named after the project and the time: *Night drive 2026-11-14 01.32*. A project you haven't saved yet is called *Untitled*. The MIDI file is named after the project too.

| File | What it is |
|---|---|
| *Mixdown.wav* | The master, after the master's safety limiter: what the speakers get. Stereo, 24-bit, at 44.1 kHz (the rate MPFree runs at). It isn't normalised or dithered, so the levels are exactly as you mixed them. |
| One WAV per stem | Each part of the mixer on its own. Every stem starts and ends with the mixdown, so they line up when you drop them into a computer. A part that made no sound at all in the export is left out, and the panel says so. |
| *Night drive.mid* | The notes and controls each track played, as they came out (with quantise, swing, chance and ratchets applied), with the tempo and the meter. It is made only when notes played. |

At the end of the range the song stops, so nothing after it is played, and the export keeps going until the sound has died away (a note's release, a delay's last echo, a reverb's tail), for up to 10 seconds. The silence after that is cut off. The range itself is never cut short.

File names work on any computer: a character that Windows or a FAT stick refuses (such as / or ?) becomes a -, and two tracks with the same name become *Drums* and *Drums 2*.

## When an export stops

If you cancel, or the export can't finish (the stick is full or too slow, or an effect that delays the sound, such as a limiter with look-ahead, is added or taken out while it runs), the export stops and removes the files it had started. The panel says why.

> [!MPC]
> On the MPC, Export as audio makes a mixdown or stems from a sequence or song in real time or faster. MPFree makes the mixdown, the stems and a MIDI file in one go, and listens on its own when the song plays outside gear.
