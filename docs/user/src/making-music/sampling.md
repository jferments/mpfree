<!--
status: changing
covers: rust/mpfree-sampler rust/mpfree-sampler-ui rust/mpfree-grain rust/mpfree-app/src/sample_page.rs rust/mpfree-app/src/sampler.rs
checked: 2026-11-12 ac6fc10
-->

# Sampling

**Sampling** means taking a recording (a drum break, a chord from a record, your own voice) and playing it from the pads. The **Sample** tab is where you shape it: trim it, chop it into pieces across the pads, and decide how each pad plays.

The Sample tab always works on one pad at a time. **Hit a pad** to pick it; its name and sound are at the top left of the page. The tab has a page for each job, along the top right: **Pads**, **Wave**, **Chop**, **Play**, **Grain** and **FX**.

> [!NOTE]
> Recording a new sample from the inputs or from MPFree's own sound onto a pad is coming to the Sample tab soon. Until then, loops recorded with the looper and the sounds in your kits are what you sample from.

## Pads: which sound is on which pad

{{shot: sample-pads | The Pads page: the kit's pads on the left, the picked pad's settings in the middle, and the sounds in the kit on the right.}}

The **Pads** page shows the 16 pads of the bank and what each holds. Tap a pad to pick it (tap again to hear it). For the picked pad you set its **sound**, how it plays (**Mode**: **One shot** plays to the end, **Gate** only while you hold the pad, **Toggle** starts and stops with each hit, **Loop** repeats its loop region, **Granular** plays it as grains), how many notes can ring at once (**Polyphony**), a **mute group** (pads in the same group cut each other off, like an open and a closed hi-hat), and its **level**, **pan** and **tune**. **Copy**, **Paste** and **Clear** move pads around; **Put on** places the sound picked on the right onto the pad.

## Wave: trim and loop

{{shot: sample | The Wave page: the whole sample at the top, the zoomed region below, and the region's settings on the right.}}

The **Wave** page shows the sample. Set where it **starts** and **ends**, and where its **loop** starts and ends, with the four Q-Links. Turn the data wheel to zoom in. Tap the wave to hear it. On the right, set the region's **gain**, **tune**, **crossfade** (smooths a loop's seam), **warp** (play in time with the tempo) and **reverse**.

## Chop: slice across the pads

{{shot: sample-chop | The Chop page: the break cut into 15 slices at its hits, ready to put on pads A2 to A16.}}

**Chopping** cuts a sample into slices, one per pad, so you can replay a drum break in a new order or pick out single hits.

1. Pick a **Method**: **Transient** cuts at each hit, **Beat** on the beats, **Grid** into equal pieces, **Silence** at the gaps, and **Manual** leaves the cuts to you.
2. Turn **Sensitivity** up for more slices, down for fewer.
3. Tap a slice to hear it. **Split** and **Merge** add or remove a cut by hand; the Q-Links move the picked slice's start.
4. Tap **Chop to** to put the slices on the pads, starting at the first pad you chose.

### Exporting a pad as files

With **Put on** at **Files**, the button reads **Export** (**Export 15 slices**, or **Export the pad** when there are no cuts) and, instead of filling pads, it saves the pad as files in a folder of its own under **Samples › Exports** in the browser:

- **Break 92.wav**: the pad's whole sound. Its loop and slice points are inside it too, where most samplers and audio editors find them.
- **Break 92.mpfree**: the pad's settings, a small text file beside the sound: its trim, loop, warp, envelope, filter, layers and slices.
- **Break 92 01.wav**, **Break 92 02.wav** and so on: each slice as a file of its own, to use anywhere.

Load **Break 92.wav** back, on this MPC or another, and the pad comes back exactly as you exported it, slices and all, as long as **Break 92.mpfree** is beside it. Without it, the sound loads on its own.

> [!NOTE]
> **Files** appears on the Put on row once MPFree can save to its stick, which comes with the browser (see [Saving your work](saving.md)).

## Play: how a pad sounds

{{shot: sample-play | The Play page: velocity layers, the volume envelope drawn over the sound, and the filter.}}

The **Play** page shapes how the picked pad sounds each time you hit it:

- **Layers:** different sounds for soft and hard hits. Drag a layer's edges to set its velocity range.
- **Envelope:** how the volume rises and falls: **attack**, **hold**, **decay**, **sustain** and **release**. Drag the points on the drawing, or use the Q-Links.
- **Filter:** darken or thin out the sound. Drag the dot.

## Grain: stretch and smear

{{shot: sample-grain | The Grain page: tiny grains of the sound play at once, around the position you pick.}}

**Granular** playback plays the sound as a cloud of tiny overlapping pieces (grains). It can freeze a moment, stretch it into a pad, or scatter it into texture. Touch the square to move the **position** in the sound and the **size** of the grains together, and use the Q-Links for **position**, **size**, **density** and **pitch**. **Scan speed** moves through the sound on its own.

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

### Crossfades

Where one zone ends and the next begins, the sound can switch at a hard edge or blend from one sample into the other. The same goes for a pad's velocity layers, from soft to hard.

- **Key fade**, a row of the Pad list, sets the blend for the whole kit: every keygroup fades in and out over that many notes, and where two zones meet they are moved to overlap by as much, so one fades out while the next fades in. The level stays even across the blend.
- **Vel fade** does the same for the picked pad's velocity layers. Round robins keep taking turns while their layers blend.
- Set either one back to Off and the edges go back to where they met. Each change is one UNDO.

The keyboard and the layer map draw each fade as a curve at the ends of its bar.

{{shot: sample-keys-crossfade | Key fade at 4 notes: the zones that met now overlap and blend over four keys.}}

> [!MPC]
> On an MPC a keygroup instrument is a program type of its own. In MPFree keygroups are pads of an ordinary kit, so any kit can become one with Map to keys.

> [!MPC]
> The MPC's Sampler, Sample Edit and Program Edit modes are all on one tab in MPFree, always on the pad you last hit. **SAMPLER** opens the Sample tab, **SAMPLE EDIT** its Wave page, and **PROG EDIT** its Pads page (in the next build). **UNDO** takes back any edit on the Sample tab.
