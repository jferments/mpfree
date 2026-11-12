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
  - After those come your own tuning files, by file name (see below).
- **Tuning root** is the note the tuning is built on. Just, Pythagorean, meantone and your own files sound purest in keys near their root, so set it to your song's key. The two equal tunings sound the same in every key and show "C (not used)".
- **A4** is the pitch of the A above middle C, from 380 to 480 Hz, so the synth can match a recording or another instrument. 432 Hz and 442 Hz are common choices.

The drum synth keeps its own pitches whatever you pick here.

### Your own tunings

MPFree reads Scala tuning files, the format thousands of published tunings come in. Put them on the MPFREE-DATA partition of your MPFree stick, in the folder `files/tunings/`, and start MPFree. They join the Tuning row after the built-in tunings, in name order, up to 64.

- **A scale** is a `.scl` file. Each step up the keyboard (each pad up, with the Chromatic pad layout) plays the next note of the scale, starting from the Tuning root, and the root keeps its usual pitch for the A4 you pick, just as with the built-in tunings. A scale needn't have 12 notes: a 19-note or a 7-note scale works, and so does one that repeats at something other than the octave.
- **A keyboard map** is a `.kbm` file with the same name beside the scale (`dorian.scl` and `dorian.kbm`). It says which key plays which note of the scale and what pitch the tuning sits at, so with one the Tuning root and A4 rows show "set by its .kbm".
- **Names** must be lower case: letters, digits, `.`, `_` and `-` only, such as `werckmeister3.scl`. A file named any other way isn't seen. The row shows at most 20 characters of a name.
- MPFree only reads the folder; it never changes your files. A file it can't read is left out, and the Console (see [Controls](../reference/controls.md)) says which and why, and lists the files it found.
