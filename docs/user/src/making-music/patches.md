<!--
status: changing
covers: rust/mpfree-patch rust/mpfree-app/src/patch_in.rs rust/mpfree-app/src/patch_page.rs rust/mpfree-app/src/patch_screen.rs rust/mpfree-app/src/gui/patches.rs
checked: 2026-11-23 9465243
-->

# Patches: splits and layers for your keyboard

A **patch** is a way of setting up your keyboard for a song: a bass under your left hand and a piano under your right, strings that come in over the piano only when you play harder, a lead an octave up from middle C. A song keeps its patches, and you switch between them while you play.

A song with no patches plays your keys as before: the armed track, and the routes on **Mix > I/O > Routing** (see [Playing several tracks from your keyboards](midi-and-cv.md#playing-several-tracks-from-your-keyboards)).

{{shot: play-keys | Play with KEYS PLAY on the transport bar: the keys play patch 3, Chorus split, and the strip under its name shows its three zones across the keyboard.}}

## What a patch is

A patch has up to 8 **zones**. Each zone takes a range of keys and a range of velocities and plays one of the song's instruments: a track's sound, or a synth no track has. Two zones over the same keys **layer**: both play. Keys no zone takes play nothing, unless you say otherwise (see [This patch](#this-patch)).

While the song has a patch, **KEYS PLAY** on Play's transport bar names the one your keys play, with a strip of its zones in each instrument's colour, so you can see the split at a glance. It sits where the bar dots were; the Sequencer tab and Arrange keep the dots. A song opens on its first patch.

## Switching patches

Tap **KEYS PLAY** to open the patch list in place of Play's view. Each patch is a tile with its number, its name, its zones over a small keyboard and the instruments they play. The one playing is outlined and marked **PLAYING**.

{{shot: play-patches | The patch list: Keys as routed, then the song's five patches. Chorus split plays; Three-way's top zone plays a track the song doesn't have, so its tile says which keys play nothing.}}

- **Tap a tile** and the keys play that patch at once; the list closes. **Keys as routed**, the first tile, plays your keys as a song with no patches does.
- **- and +** pick the patch before or after while the list stays open; the wheel moves the outline and pushing it picks.
- **Close**, or **KEYS PLAY** again, closes the list with nothing changed.

A switch never cuts a note: a key you hold, and a chord under the sustain pedal, keep sounding on the patch they started on until you let go. If you'd rather they stop when the new patch comes in, turn on **Cut the old sound** for that patch.

If a zone plays an instrument the song doesn't have any more (a track you deleted, say), its tile says so in red, with the keys that play nothing, and **KEYS PLAY** shows a red dot while that patch plays.

## Making a patch

1. Open the patch list: tap **KEYS PLAY**, or, while the song has no patches yet, press **MENU** on Play and pick **Patches**. Tap **New patch**.
2. Pick a layout: **Whole keyboard**, **Bass + keys** (split at C3), **Two layered**, or **Three-way split** (up to B2, C3 to B4, C5 up). Every zone starts on the instrument your keys play now.
3. The editor opens on the new patch, and your keys play it. Pick what each zone plays, and set its keys.

To start from a patch you have, open it and tap **Copy**: the copy goes after it, named "Chorus split copy". Tap the name at the top of the editor to give it a name of its own, up to 32 characters.

## The patch editor

Tap **Edit** on a patch's tile. Your keys play that patch while you edit it, so what lights up is what you hear.

{{shot: play-patch-edit | The editor on Chorus split: bass under C3 that ignores the sustain pedal, a grand piano from C3 up, and strings over the piano from velocity 64. C2 is held softly and E4 and G4 at 96, so the three keys light and so do the zones they reach.}}

- **The zones** are coloured bars over a keyboard, one row each, with each zone's velocity range on the right (**VEL**). Press keys and they light up on the keyboard, with the zones they reach; their velocity shows on each zone's velocity bar. The line under the keyboard says what came in last and what it played.
- **Tap a zone** to pick it. Drag either end of its bar along the keys to move that end, or tap **Learn low** or **Learn high** and play the key you want. A new zone comes with **Learn low** lit, so you can play its lowest key straight away.
- **+ Zone** adds a zone over every key, playing what the last zone plays. **Remove** takes the picked zone out, as does holding **ERASE** and tapping its row.

### The picked zone

| Row | What it does |
|---|---|
| **Plays** | The instrument: one of the song's tracks, or a synth no track has, named as on Mix. |
| **Keyboard** | Which keyboard the zone listens to: any keyboard, or one, on all its channels or one channel. |
| **Low key**, **High key** | The keys the zone takes. |
| **Transpose** | Moves its notes by semitones, up to 48 either way. |
| **Low velocity**, **High velocity** | Which velocities the zone takes. Strings from 64 to 127 come in only when you play harder. |
| **Velocity** | How it plays the velocities it takes: **As played**, **Soft**, **Hard** or **Fixed 100**. |
| **Pedals** | On: the sustain, sostenuto and soft pedals reach this zone. Off for a bass that shouldn't ring on. |
| **Wheels and pressure** | On: the pitch wheel, mod wheel, aftertouch and MPE reach this zone. |

### This patch

| Row | What it does |
|---|---|
| **Keys outside the zones** | **Nothing** (the default), **Play an instrument** (pick which), or **As routed** (the armed track and your routes, as before). |
| **Cut the old sound** | Off: held notes and pedalled chords ring on the patch they started on. On: they stop when this patch comes in. |

Turn the wheel to pick a row and press **-** or **+** to change it, or drag the row. The **Q-Links** turn the picked zone's Low key, High key, Transpose and Low velocity.

Every change is one step of **UNDO**, and **UNDO** brings back a zone or a patch you removed. **Delete** asks first: tap it again to delete the patch.

## What's coming

Switching patches from a footswitch (**Next patch** and **Previous patch** with [MIDI Learn](midi-and-cv.md#midi-learn)) or with a program change from your keyboard is coming next, each able to wait until your hands are up. After that: scenes and song sections that pick a patch, recording with a patch (one track per zone), and pads and knobs that change with the patch.

> [!MPC]
> On the MPC, a keyboard plays the track that's selected, or the tracks set to its MIDI input. In MPFree a patch splits and layers one keyboard over several of the song's instruments, and you switch the whole setup with one tap.
