<!--
status: changing
covers: rust/mpfree-app/src/app.rs rust/mpfree-controller
checked: 2026-11-11 414faa1
-->

# A tour of the MPC One

If you have never used an MPC, this chapter introduces the hardware: what each part of the panel is for, and what it does in MPFree. The exact job of every button is listed in [Buttons and controls](../reference/controls.md).

## The pads

The 16 square pads are what you play. Each one plays a sound: a drum, a slice of a sample, or a note of a synth. They feel how hard you hit them (velocity) and how hard you keep pressing (pressure), so a soft tap plays quietly and a hard hit plays loud.

The pads are numbered 1 to 16, starting at the bottom left and going across, row by row, to 16 at the top right.

**Pad banks.** Sixteen sounds are not always enough, so the four **PAD BANK** buttons (A, B, C, D) switch the pads to another set of 16. Hold **SHIFT** while pressing one, or press it twice, to reach banks E to H. A drum kit can have a different sound on every pad in every bank.

**Full Level.** Press **FULL LEVEL** when you want every hit at full volume, however you play; press it again to go back. **SHIFT + FULL LEVEL** does the same at half volume.

## The screen

The large touchscreen shows what MPFree is doing and lets you change it with your fingers: tap, drag and hold. [The screen](the-screen.md) explains its layout.

Around the screen:

- The **data wheel** (the big dial) changes whatever is selected. On the Play page it changes the drum kit.
- The **–** and **+** buttons beside it step a value down or up.
- **UNDO** undoes your last recording or edit; **SHIFT + UNDO** redoes it.
- **SHIFT** gives many buttons a second job, printed in red under their name on the panel. Hold it, then press the other button.

## The Q-Link knobs

The four knobs in a column beside the screen are the **Q-Links**. They are "quick links" to whatever matters most on the page you are on: swing, a filter, an effect's amount, a loop's level. The screen shows what each knob controls in a matching column right beside them, with knob 1 at the top. The knobs are touch-sensitive: rest a finger on one and its card lights up.

## The transport buttons

Along the bottom right are the buttons that control time, like the buttons on a tape recorder:

- **PLAY** starts playing from where you stopped; **PLAY START** starts from the beginning.
- **STOP** stops. Press it again to silence every sound still ringing.
- **REC** records what you play.
- **OVERDUB** records on top of what is already there, without erasing it.
- **TAP TEMPO**: tap it in time with the music to set the tempo.

## The mode buttons

The buttons above the screen, such as **MENU**, **MAIN**, **STEP SEQ**, **SAMPLER** and **TRACK MIX**, jump to a part of MPFree. Two you will use all the time:

- **MENU** opens the options for the page you are on.
- **SHIFT + MENU** opens **Prefs**, MPFree's settings.

**NOTE REPEAT** makes a pad repeat in time while you hold it; see [Note Repeat, Latch and the arpeggiator](../making-music/note-repeat-and-arp.md).

## The connectors

On the front:

- **PHONES**: headphones (a 3.5 mm mini-jack).

On the back:

- **OUTPUTS**: two 6.35 mm (1/4") jacks for speakers, a mixer or an audio interface. The **MASTER VOL** knob sets their volume and the headphones'.
- **INPUTS**: two 6.35 mm jacks to record from a synth, a turntable or another instrument. The **REC VOL** knob sets how loud they come in. In MPFree the inputs are off until you turn them on in Prefs (Line input).
- **MIDI IN** and **MIDI OUT**, **USB** ports, **CV/Gate** outputs and the **Link** (Ethernet) port: MPFree doesn't use these for music yet.

> [!MPC]
> The hardware is the same, but a few buttons do different jobs in MPFree. **MAIN** goes to the Play page and, pressed again, switches its view. **SHIFT + MENU** opens MPFree's Prefs. The Q-Links always follow the page you are on, so there is no Q-Link Edit mode. See [Buttons and controls](../reference/controls.md) for every difference.
