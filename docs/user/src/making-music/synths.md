<!--
status: changing
covers: rust/mpfree-synth rust/mpfree-synth-ui rust/mpfree-app/src/synth.rs
checked: 2026-11-11 414faa1
-->

# Synths

MPFree's built-in synths, playing them from the pads, and pads in key.

The rest of this chapter is not written yet. It will cover:

- Poly, FM, Wavetable, Drums and Pluck
- Presets
- Playing the synth on the pads: Chromatic and In key
- Pad pressure

## Tuning

Out of the box every synth plays in standard tuning: twelve equal steps to the octave, with the A above middle C at 440 Hz. You can change that for the synth on the pads in the Synth tab's options (press MENU on the Synth tab; see [Controls](../reference/controls.md)).

{{shot: synth-options | The Synth tab's options, with the Tuning, Tuning root and A4 rows.}}

- **Tuning** picks how the notes are tuned:
  - **12-TET** is standard tuning, the same in every key.
  - **Just (5-limit)** makes thirds and fifths beat-free and sweet, in keys near its root.
  - **Pythagorean** has pure fifths, for drones and open, medieval-sounding chords.
  - **Meantone 1/4 comma** (quarter-comma meantone) has pure major thirds, as on Renaissance and early Baroque keyboards.
  - **24-TET quarter tones** has 24 equal steps to the octave, so each step up is half a semitone.
- **Tuning root** is the note the tuning is built on. Just, Pythagorean and meantone sound purest in keys near their root, so set it to your song's key. The two equal tunings sound the same in every key and show "C (not used)".
- **A4** is the pitch of the A above middle C, from 380 to 480 Hz, so the synth can match a recording or another instrument. 432 Hz and 442 Hz are common choices.

The drum synth keeps its own pitches whatever you pick here.
