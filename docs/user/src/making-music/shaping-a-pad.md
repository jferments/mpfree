<!--
status: changing
covers: rust/mpfree-sampler/src/model.rs rust/mpfree-sampler-ui/src/play_tab.rs rust/mpfree-sampler-ui/src/grain_tab.rs rust/mpfree-sampler-ui/src/keys_widgets.rs rust/mpfree-grain rust/mpfree-app/src/sample_page.rs
checked: 2026-11-14 b76c3c7
-->

# Shaping how a pad plays

Once a sound is on a pad (see [Sampling](sampling.md)), the Sample tab's **Play**, **Grain** and **FX** pages decide what happens when you hit it: how loud and how bright, which other pads it stops or brings in, whether it plays as grains, and which effects it goes through. The last part of this chapter turns a whole kit into a keyboard instrument.

As everywhere on the Sample tab, **hit a pad** to pick the one you are shaping.

## Play: layers, envelope and filter

{{shot: sample-play | The Play page: velocity layers, the volume envelope drawn over the sound, and the filter.}}

The **Play** page shapes how the picked pad sounds each time you hit it:

- **Layers:** different sounds for soft and hard hits. Drag a layer's edges to set its velocity range.
- **Envelope:** how the volume rises and falls: **attack**, **hold**, **decay**, **sustain** and **release**. Drag the points on the drawing, or use the Q-Links.
- **Filter:** darken or thin out the sound. Drag the dot.

### Pads that stop, play or hold others

{{shot: sample-play-links | The Play page's Pad list with Also plays picked: hits on A1 and A3 put them on it, so a hit on A5 plays all three, and A5 latches.}}

Three rows of the Play page's **Pad** list set how a pad works with the others, and with your hands:

- **Stops:** the pads this one cuts off when you hit it. It works one way: the closed hi-hat stops the open one, and the open one leaves the closed one alone. (A **mute group** cuts both ways.)
- **Also plays:** the pads that play whenever you hit this one, at the same strength, as the MPC's Simultaneous Play does: a kick with its own sub, a stab with a noise burst, a chord from three pads. Each plays as a hit on it would, with its own layers, stops and latch, and lets go when you let go of this pad. The pads it plays don't play their own Also plays pads in turn. While the pads play notes or 16 Levels, Also plays does nothing.
- **Latch:** at **On**, a hit starts the pad and it plays on after you let go; the next hit stops it. A loop keeps looping and plays its tail when it stops, a grain cloud keeps going, a long pad or a drone holds, and your hands are free for the rest. A latched pad stays lit on the MPC.

To fill in **Stops** or **Also plays**, tap the row, then hit the pads you want: each hit adds a pad, and a second hit takes it out again. Every hit is one step for **UNDO**. Tap another row when you are done, and hits pick the pad to edit again. The arrows and **–** and **+** work too (and the wheel, once you push it): a step up adds the next pad, a step down takes the last one out.

> [!TIP]
> Note Repeat plays a pad again on every step, so a latched pad would turn on and off with each one. Keep Latch for pads you hold, and Note Repeat for pads you roll.

**FULL LEVEL** and Half Level play every hit at full or at half strength, sampled pads included (see [Buttons and controls](../reference/controls.md)).

### Scenes: mangle, morph and go back

{{shot: sample-play-scenes | The scene strip on the Play page: scenes A and B stored, the crossfader 40 % of the way to B.}}

The strip above the Q-Links on the **Play** page holds two **scenes** of the whole kit, as on the Octatrack:

- **Store A** keeps every pad of the kit as it is now: its sound, trim, level, tune, filter, envelope, everything. **Store B** keeps a second version. Make A, change some pads (tune the snare down, close the hats' filter, push the kick), then make B.
- The **crossfader** morphs the kit from A to B as it plays: each pad's level, pan, tune, filter and envelope times move between their values in A and in B. Settings that are the same in both scenes stay as they are. Sliding it changes nothing in the kit itself, so you can perform with it freely.
- **Return to A** puts every pad back exactly as A stored it and sets the crossfader to A. Mangle the kit as much as you like, then return. **UNDO** takes the return back.

The scenes are saved with the kit.

## Grain: stretch and smear

{{shot: sample-grain | The Grain page: tiny grains of the sound play at once, around the position you pick.}}

**Granular** playback plays the sound as a cloud of tiny overlapping pieces (grains). It can freeze a moment, stretch it into a pad, or scatter it into texture. Touch the square to move the **position** in the sound and the **size** of the grains together, and use the Q-Links for **position**, **size**, **density** and **pitch**. **Scan speed** moves through the sound on its own.

Grains play only when the pad's **Mode** is **Granular**. Until then a button over the picture says which mode the pad is in: tap it to play the pad as grains. The **Mode** row steps through every mode to **Granular** too (on the way, **Loop** gets a loop region of its own, as on the Wave page).

## FX: effects on one pad, or on the whole kit

**FX**, in the Sample tab's row of pages, opens the **chain editor** (see [Mixing and effects](effects.md#adding-an-effect-the-chain-editor)) on the pad you are editing, in the Sample tab's place. **Back** comes back to the Sample tab.

- **The pad's own effects.** The editor opens on the pad's own channel, so an effect added there plays on that pad alone. Add **Perform FX** for stutters, beat repeats, tape stops and reverse on just that pad, in time with the tempo. Each of its 16 slots (**Slot 1** to **Slot 16**) is a page in the editor, with its effect, its length and a **Hold** switch: Hold on plays it, off lets it go.
- **The whole kit's effects.** The switch at the top goes to **Drums**, the track every pad plays into, so an effect there plays on the whole kit, like a bus effect on the SP-404. To make it a one-knob effect, put its knobs on a macro (see [Macros](effects.md#macros)), and that macro's Q-Link plays them all at once.

The effects belong to the pad's place in the kit, not to its sound: put another sound on the pad and it plays through the same effects. Only bank A's 16 pads have a channel of their own for now. **FX** on a pad past them says so in the log.

## Playing a kit on keys

A kit's pads can play notes instead of their own sounds, so a bank of sampled notes plays like a keyboard instrument. A pad that plays notes is a *keygroup*: it covers a range of keys and plays its sound higher or lower for each key away from its root note.

**Map to keys**, under the pads on the Pads tab, does this for the whole bank at once. MPFree finds each pad's note: from its keygroup if it is one already, else from a note in its name ("Pluck C3", "Bass_F#1"), else from the pitch it hears in the sound. It then spreads the pads over the keyboard, each one reaching halfway to its neighbours, and the pads play notes from the C at or below the lowest. A pad with no note to find is left as it was. One UNDO takes the whole mapping back.

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
