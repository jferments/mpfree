<!--
status: planned
covers: rust/mpfree-midi
checked: 2026-11-12 7ecb7e4
-->

# Playing other gear: MIDI, sync and CV

The **MIDI IN** and **MIDI OUT** jacks, a USB keyboard on the **USB** port and the **CV/Gate** outputs connect MPFree to other instruments. MPFree doesn't use them yet. This page says what is coming, so you can plan your setup.

## Playing MPFree from a keyboard

Plug a keyboard into **MIDI IN**, or a USB keyboard into the **USB** port, and it plays the track on screen, just as the pads do. Its knobs and pedals can be learned (see MIDI Learn below).

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

With **Show developer tools** on in [Prefs](../reference/prefs.md), the Dev tab's MIDI page lists every message in and out. It already shows the MPC's own pads, knobs and buttons; the MIDI jacks and USB keyboards are coming.
