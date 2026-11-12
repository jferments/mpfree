<!--
status: changing
covers: rust/mpfree-midi rust/mpfree-midi-ui/src/learn.rs rust/mpfree-app/src/midi_in.rs rust/mpfree-app/src/cv_out.rs
checked: 2026-11-12 34d34b4
-->

# Playing other gear: MIDI, sync and CV

The **MIDI IN** and **MIDI OUT** jacks, a USB keyboard on the **USB** port and the **CV/Gate** outputs connect MPFree to other instruments. Playing MPFree from a keyboard and [MIDI Learn](#midi-learn) work now. Following another device's clock works too, once the I/O view is on the MPC's screen. Playing gear from **MIDI OUT** and the CV outputs stay off until they've been checked on an MPC One. This page says what each will do, so you can plan your setup.

## Playing MPFree from a keyboard

Plug a keyboard into **MIDI IN**, or a USB keyboard into the **USB** port, and it plays the armed track, just as the pads do. Its knobs, faders and pedals can turn effect knobs with [MIDI Learn](#midi-learn). It listens on every MIDI channel, and you can plug a USB keyboard in or out while MPFree runs.

- **With a synth on the pads,** each key plays its own note, through the arpeggiator, so **NOTE REPEAT** and Latch work from the keyboard too. A note held on a key and a pad ends when both let go.
- **With a drum kit,** the keys that match the MPC's own pad notes hit the pads: C#2 is pad 1, C2 is pad 2, and so on. Other keys play nothing.
- The pad velocity setting, and Full Level, shape the keys' velocity as they do the pads'.

> [!MPC]
> On the MPC, MIDI input can also be set to play a track of its choosing per channel. In MPFree a keyboard plays the armed track; playing several tracks from one keyboard is coming.

## Playing a synth from a track

A track can play a synth or drum machine through **MIDI OUT** instead of a sound inside MPFree. It has one setting for where it plays: the port and the MIDI channel together. A track can also send a program change, so the synth switches to the right sound when the song starts.

If a track would send nothing, because no port is picked or the device is unplugged, it says so under the track's name.

> [!MPC]
> On the MPC, a MIDI track needs two settings with nearly the same name before a note leaves the jack: the track's MIDI output channel and its MIDI Out routing. In MPFree it's one setting, the port and the channel together.

## Staying in time with other gear

MPFree can follow another device's MIDI clock. In the mixer's I/O view, under **Sync**, set **Tempo from** to the port the device is plugged into:

- Its tempo becomes MPFree's, on the tempo chip too, and moves when it moves.
- With **Start and stop** on, its Start and Stop start and stop MPFree.
- With **Position** on, MPFree plays from the same bar as the device, and goes back in step if it drifts away.
- If its clock stops arriving, MPFree keeps playing at the last tempo.

MPFree can also send its own clock from **MIDI OUT**, so a drum machine or an arpeggiator plays in time with it. Each port's **Clock out** is **Off**, **While playing** or **Always**.

The Sync settings come to the MPC's screen with the I/O view.

## MIDI Learn

MIDI Learn lets a knob, fader, button, key or pedal on your keyboard or controller turn an effect's knob.

1. Open the effect in the [chain editor](mixing-and-effects.md#the-chain-editor) and touch the knob.
2. Tap **Learn to a Q-Link**. It now reads **Turn a Q-Link or a MIDI control**.
3. Move the control on your keyboard. From now on it turns that knob, as far up and down as the knob goes. (Turn a Q-Link instead, and the Q-Link learns it, as before.)

The move that teaches a control doesn't play a note, and neither does a learned control. Teach a control again and it moves to the new knob.

Each learned control is listed in the mixer's I/O view, under **MIDI** and then **MIDI Learn**, with the knob it moves. **Works as** says how it moves it:

- **Knob or fader**: the knob follows it. Every control but a key starts as this.
- **Endless**: for a knob that turns forever. Each click moves the knob one step on from where it is, even after you changed it on the screen. Controllers send these three ways, so if the knob jumps or runs backwards, try the next one: **1 up, 127 down**, **65 up, 63 down** or **1 up, 65 down**.
- **Button: on, then off**: each press switches between the knob's top and bottom.
- **Button: while held**: top while held, bottom when let go. A key starts as this.

Tap **Forget** to take a learned control off. A row in amber says why its control does nothing: the device isn't plugged in, or its port is set to **Plays tracks**, which keeps every message for playing. A port set to **Learn only** turns knobs but plays no notes.

Still to come: the I/O view on the MPC (until then MIDI Learn works, but the list isn't shown), keeping learned controls with the project (for now they last until MPFree quits), learning any value on the screen rather than only effect knobs, a range for each control so a fader's top can stop at 0 dB, and a control that waits until it reaches the knob's value before taking over.

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
