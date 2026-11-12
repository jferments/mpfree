<!--
status: changing
covers: rust/mpfree-midi rust/mpfree-app/src/midi_in.rs rust/mpfree-app/src/cv_out.rs
checked: 2026-11-12 d5f420a
-->

# Playing other gear: MIDI, sync and CV

The **MIDI IN** and **MIDI OUT** jacks, a USB keyboard on the **USB** port and the **CV/Gate** outputs connect MPFree to other instruments. Playing MPFree from a keyboard works now. The rest of this page is coming, so you can plan your setup.

## Playing MPFree from a keyboard

Plug a keyboard into **MIDI IN**, or a USB keyboard into the **USB** port, and it plays the armed track, just as the pads do. It listens on every MIDI channel, and you can plug a USB keyboard in or out while MPFree runs.

- **With a synth on the pads,** each key plays its own note, through the arpeggiator, so **NOTE REPEAT** and Latch work from the keyboard too. A note held on a key and a pad ends when both let go.
- **With a drum kit,** the keys that match the MPC's own pad notes hit the pads: C#2 is pad 1, C2 is pad 2, and so on. Other keys play nothing.
- The pad velocity setting, and Full Level, shape the keys' velocity as they do the pads'.

The keyboard's knobs and pedals are coming with MIDI Learn (below).

> [!MPC]
> On the MPC, MIDI input can also be set to play a track of its choosing per channel. In MPFree a keyboard plays the armed track; playing several tracks from one keyboard is coming.

## Playing a synth from a track

A track can play a synth or drum machine through **MIDI OUT** instead of a sound inside MPFree. It has one setting for where it plays: the port and the MIDI channel together. A track can also send a program change, so the synth switches to the right sound when the song starts.

If a track would send nothing, because no port is picked or the device is unplugged, it says so under the track's name.

> [!MPC]
> On the MPC, a MIDI track needs two settings with nearly the same name before a note leaves the jack: the track's MIDI output channel and its MIDI Out routing. In MPFree it's one setting, the port and the channel together.

## Staying in time with other gear

MPFree can follow another device's MIDI clock, so its tempo, start and stop come from there. It can also send its own clock, so a drum machine or an arpeggiator plays in time with it. You choose which device sets the tempo, and which ports send clock.

## MIDI Learn

MIDI Learn lets a knob, fader, key or pedal on your controller move any value on the screen: pick the value, choose Learn, and move the control. Each learned control can have a range, so a fader's top can stop at 0 dB instead of the loudest setting. A learned control follows the sound it moves, so moving tracks around doesn't break it.

## CV and gate

The **CV/Gate** jacks play modular synths. Each of the eight outputs has one job:

- a track's **pitch**, 1 V per octave (or Hz per volt, for older Korg and Yamaha synths), with the lowest, highest or last note winning when several are held;
- a track's **gate**, high while a note is held, or a short **trigger** on each note (of every note, or of one note, for a drum module);
- a track's **velocity**, or one of its **CCs**, as a voltage;
- a **clock** at the rate you pick, **run** (high while the song plays) or **reset** (a pulse when the song starts from the top);
- or a **fixed** voltage.

The outputs are set in the mixer's I/O view, under CV, and are kept with the project. A track that drives CV still plays its own sound too, so turn it down in the mixer if you only want the module.

When one note ends just as the next starts, as a sequence's notes do, the gate drops for 2 ms between them, so the module's envelope starts again. Hold two notes at once for a legato line instead.

CV 1 to 4 will work first, once MPFree has been checked on an MPC One with a meter; CV 5 to 8 come after that check. Until an output works, its row in the I/O view says it sends nothing. Modules differ, so each output will also be tunable against your module.

## Seeing what comes in and goes out

With **Show developer tools** on in [Prefs](../reference/prefs.md), the Dev tab's MIDI page lists every message in, from the MPC's own pads, knobs and buttons, from **MIDI IN** and from USB keyboards. Clock from another device is counted rather than listed: the page says when it starts and stops arriving.
