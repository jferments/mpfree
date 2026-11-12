<!--
status: changing
covers: rust/mpfree-midi rust/mpfree-app/src/midi_in.rs
checked: 2026-11-12 b4f1d46
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

The **CV/Gate** jacks play modular synths: a track sends a pitch voltage and a gate, and other outputs can send a value that moves slowly, like a knob. Modules differ, so each output can be tuned against your module before you rely on it.

## Seeing what comes in and goes out

With **Show developer tools** on in [Prefs](../reference/prefs.md), the Dev tab's MIDI page lists every message in, from the MPC's own pads, knobs and buttons, from **MIDI IN** and from USB keyboards. Clock from another device is counted rather than listed: the page says when it starts and stops arriving.
