<!--
status: changing
covers: rust/mpfree-export rust/mpfree-browser-ui/src/export.rs rust/mpfree-app/src/export.rs rust/mpfree-fx/src/bounce.rs rust/mpfree-fx/src/resample.rs
checked: 2026-11-14 97a6b00
-->

# Exporting your music

**Export** turns your song into files you can take anywhere: a finished stereo mix to share, each track on its own (stems) to mix in a computer, and a MIDI file of the notes. One export makes all three in one go, lined up to the sample, in a new folder on the stick.

## Making an export

1. Press **BROWSE**, pick **Projects** on the left and press **Export**. The Export panel opens over the browser.
2. Tick what to make (all three are ticked the first time; after that the panel remembers your last choice until MPFree restarts):
   - **Mixdown**: the whole song in one stereo file, as the speakers play it.
   - **Stems**: a stereo file for each track (or each part of the mixer: see the next step), after its fader, pan and mute.
   - **MIDI file**: the notes and controls each track played.
3. Pick what a stem is, under **Stems by** (see [Stems by track or by channel](#stems-by-track-or-by-channel)):
   - **Track** (the default): one stem per track, as the MPC makes them.
   - **Channel**: one stem per part of the mixer, in a single pass.
4. Pick the **Range**:
   - **Arrangement**: the song on Arrange, from its start to its end. It is picked for you when the song has one.
   - **Bars**: the clips playing now, from the start of the bar the playhead is in, for the number of bars you set (8 to start with). Turn the data wheel, or tap **–** and **+** on the panel, to change it.
5. Check the **File** row: the WAV files' format (see [File formats](#file-formats)). It starts as Prefs has it, and what you pick here is for this export only.
6. Press **Start**, or push the data wheel.

While it runs, the panel shows how far it has got ("1:12 of 3:50", and "pass 2 of 6" when the range plays more than once). **Cancel** stops it. When it is done, the panel says what it made, and **Show in BROWSE** opens the folder.

You can close the browser (**BROWSE**) while an export runs and go on with something else: the export keeps going, and BROWSE brings the panel back.

The export starts the song from the beginning of the range and plays it once, and again for each track whose stem needs a pass of its own (see below). When it is over, the transport goes back to where it was, with the loop and Song mode you had. Pressing **STOP** before the end cancels the export.

What you play on the pads while an export runs goes into it, so leave them alone unless you mean it.

## Fast or Listen

An export plays the song in one of two ways. **Prefs > Export** picks which:

- **Fast, quiet** (the default): the song plays as fast as the MPC can make it, with the speakers quiet. How much faster than real time depends on how hard the song works the MPC: a light song is done in a fraction of its length, and one that already works the MPC hard takes about as long as it plays.
- **Listen**: the song plays in real time, and you hear it as it records (each pass, when it plays more than once).

Some songs **always listen**, because part of their sound only exists in real time. The panel says which reason applies before you start:

- a track plays outside gear over MIDI;
- the line input is on;
- the CV or gate jacks are in use;
- MIDI clock goes out to other gear;
- the tempo follows MIDI clock from other gear;
- the song uses plugins (for now).

## Stems by track or by channel

**Stems by Track** (the default) makes one stem for each track that plays, named after the track, as the MPC does:

- A track with its own instrument plays through its own channel, so its stem comes straight from the first pass.
- The tracks that play the pads all play into the same channel (the drum kit's, or the synth's when the pads play the synth). To give each its own stem, the export plays the range again with only that track playing, once for each. The audio tracks share the Audio channel and get a pass each the same way. A track that is the only one on its channel needs no extra pass.
- The Loop and each return get a stem too, as with Channel.
- A track that is muted, has no notes, or plays outside gear over MIDI gets no stem (its notes are still in the MIDI file).

The panel says how many passes it takes ("Plays the range 6 times: 5 tracks share a channel"). Each extra pass plays exactly the notes the first pass played, chance and the arps' random notes included, so the stems match the mix. A few things can still differ by a hair between passes: a sound with round-robin samples may pick another sample, and an analog-style synth's drift is never quite the same twice. An effect on a shared channel that reacts to everything in it (a compressor on the drum kit, say) hears only one track in each extra pass, so the stems of those tracks don't add up exactly to the mix; that is the same on the MPC.

If the drum kit's channel is frozen, its tracks can't be told apart, so it gets one stem, named after the channel.

**Stems by Channel** makes one stem for each part of the mixer that plays (Drums, Synth, Loop, Audio, each track's own instrument, and each return) in a single pass. Those stems add up exactly to the mix.

## What you get

The files go in a new folder in **Recordings**, named after the project and the time: *Night drive 2026-11-14 01.32*. A project you haven't saved yet is called *Untitled*. The MIDI file is named after the project too.

| File | What it is |
|---|---|
| *Mixdown.wav* | The master, after the master's safety limiter: what the speakers get. Stereo, in the format you picked (24-bit at 44.1 kHz unless you change it). It isn't normalised, so the levels are exactly as you mixed them. |
| One WAV per stem | Each track (or part of the mixer) on its own. Every stem starts and ends with the mixdown, so they line up when you drop them into a computer. A stem that made no sound at all in the export is left out, and the panel says so. |
| *Night drive.mid* | The notes and controls each track played, as they came out (with quantise, swing, chance and ratchets applied), with the tempo and the meter. It is made only when notes played. |

At the end of the range the song stops, so nothing after it is played, and the export keeps going until the sound has died away (a note's release, a delay's last echo, a reverb's tail), for up to 10 seconds. The silence after that is cut off. The range itself is never cut short.

File names work on any computer: a character that Windows or a FAT stick refuses (such as / or ?) becomes a -, and two tracks with the same name become *Drums* and *Drums 2*.

## File formats

The **File** row on the Export panel picks how the WAV files hold the sound. **Prefs > Export file** and **Prefs > Export rate** set what it starts as each time, so set those once for the computer or label you send to, and change the panel only for an odd one out.

- **24-bit** (the default): keeps everything the mixer makes, far below anything you can hear. Right for nearly everything: a computer, a mastering engineer, a streaming service.
- **16-bit**: for a CD, or older gear and software that only takes 16-bit. The quietest parts would break up at 16 bits, so a very faint hiss (dither) is added, far below anything you'd hear at a normal level, which keeps fades and reverb tails smooth.
- **32-bit float**: exactly what the mixer made, number for number. A stem whose level goes over full scale (a loud track before the master's limiter) keeps that level instead of clipping, so you can turn it down in a computer later. The files are a third bigger than 24-bit.
- **44.1 kHz** (the default): the rate MPFree plays at, written as it is.
- **48 kHz**: the rate for video. The sound is converted as the files are written; nothing you can hear changes and every file still starts on the song's first moment, lined up with the others and the MIDI file. An export at 48 kHz takes a little longer.

Each choice is said with the result, for example "3 WAV files (16-bit, 48 kHz)".

## When an export stops

If you cancel, or the export can't finish (the stick is full or too slow, or an effect that delays the sound, such as a limiter with look-ahead, is added or taken out while it runs), the export stops and removes the files it had started. The panel says why.

> [!MPC]
> On the MPC, Export as audio makes a mixdown or one stem per track from a sequence or song in real time or faster, and lets you pick the bit depth and rate. MPFree makes the mixdown, the stems (per track, or per mixer channel in one pass) and a MIDI file in one go, listens on its own when the song plays outside gear, and keeps your usual format in Prefs.
