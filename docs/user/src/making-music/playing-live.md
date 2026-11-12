<!--
status: changing
covers: rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/play_widgets.rs rust/mpfree-app/src/mixing.rs rust/mpfree-fx/src/perform.rs
checked: 2026-11-12 b9b7477
-->

# Playing live

The Play page is MPFree's home for finger drumming, live looping and making beats. This chapter covers its four views and the performance tools.

{{shot: play | The Play page: the loop strip, the pattern beside the mini pads, the throws and the XY pad, and the Q-Links on the right.}}

## Throws and the XY pad

The throws are buttons for effects you play like an instrument: hold one and the effect comes in, let go and it stops. They are Delay throw (a dotted echo), Reverb wash, Filter dip (a low-pass sweep over a bar), Stutter, Tape stop, Beat repeat and Vinyl brake. While the music plays, a throw starts on the next sixteenth note, so it lands in time even if you press a little early.

The XY pad is a filter and an echo under one finger. Left of the middle is a low-pass filter (darker), right of it a high-pass filter (thinner), and the middle is no filter at all. Higher up adds more echo. When you lift your finger the pad glides back to the middle and the bottom, so the sound comes back clean. Its label says where it is, for example "HP 173 Hz · echo 62 %".

The **FX mix** Q-Link sets how much of the throws and the XY pad you hear, from none to all.

## Fills

While the music plays, hold **PLAY** for a fill: notes set to play only in fills come in, and notes set to drop out in fills go quiet, until you let go. Press **PLAY** twice to keep the fill on; press it again (or STOP) to end it. When the music is stopped, PLAY just starts it. The exact buttons are in [Buttons and controls](../reference/controls.md).

## Muting tracks and pads

**TRACK MUTE** turns the pads into mute buttons over whatever page you are on, so you can drop parts in and out without leaving the screen you are using. Pads 1-8 are tracks 1-8: green while the track plays a clip, dim green when it has nothing playing, red while it is muted. Tap a pad to mute or unmute its track; the track's clip keeps its place, so it comes back in time. **SHIFT + TRACK MUTE** does the same for the pads of the kit: a pad lit in the kit's colour plays, a red one is muted. Press the same button again to play the pads as usual. The hint line at the bottom of the screen says which mode the pads are in.

> [!MPC]
> On the MPC, Track Mute opens a page of its own. In MPFree the screen stays on what you were doing and only the pads change.

This chapter is not finished. Still to come:

- The four views: All, Beats, Loop, Drum
- Muting pads with the speakers in the pattern, and loop layers
- Your timing: the Drum view
- Q-Links on the Play page
