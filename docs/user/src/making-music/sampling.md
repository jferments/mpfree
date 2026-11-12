<!--
status: changing
covers: rust/mpfree-sampler rust/mpfree-sampler-ui rust/mpfree-app/src/sample_page.rs
checked: 2026-11-12 b9b7477
-->

# Sampling

Record a sample, chop it across the pads, and shape how each pad plays, on the Sample tab.

This chapter is not written yet. It will cover:

- Recording a sample
- The Wave page
- Chopping: by hits, by beats, by hand
- How a pad plays
- Grain
- Pad effects

## Playing a kit on keys

A kit's pads can play notes instead of their own sounds, so a bank of sampled notes plays like a keyboard instrument. A pad that plays notes is a *keygroup*: it covers a range of keys and plays its sound higher or lower for each key away from its root note.

**Map to keys**, on the Sample tab's Pads page, does this for the whole bank at once. MPFree finds each pad's note: from its keygroup if it is one already, else from a note in its name ("Pluck C3", "Bass_F#1"), else from the pitch it hears in the sound. It then spreads the pads over the keyboard, each one reaching halfway to its neighbours, and the pads play notes from the C at or below the lowest. A pad with no note to find is left as it was. One UNDO takes the whole mapping back.

Notes are named with middle C as C4. Some libraries call middle C "C3"; when a name and the pitch MPFree hears are the same note, it uses the octave it hears.

{{shot: sample-keys-mapped | The Play page's keyboard after Map to keys: five keygroups, with pad A5's zone picked.}}

While the pad is a keygroup, or the pads play notes, the Play page shows a keyboard:

- Each keygroup is a bar over the keys it plays, in its pad's colour, with a dot on its root.
- Tap a bar to pick its pad. Drag the picked pad's ends to set its lowest and highest keys, and its dot to move the root.
- Press the keys to hear the kit. The line under the keys shows the 16 notes the pads play.

The Keys rows in the Pad list do the same with numbers: Keys makes one pad a keygroup, Low key, High key and Root key set its range, and Pads play switches the pads between their own sounds and notes.

> [!MPC]
> On an MPC a keygroup instrument is a program type of its own. In MPFree keygroups are pads of an ordinary kit, so any kit can become one with Map to keys.
