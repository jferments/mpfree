<!--
status: changing
covers: rust/mpfree-sampler rust/mpfree-sampler-ui rust/mpfree-grain rust/mpfree-app/src/sample_page.rs rust/mpfree-app/src/sampler.rs
checked: 2026-11-13 9560b32
-->

# Sampling

**Sampling** means taking a recording (a drum break, a chord from a record, your own voice) and playing it from the pads. The **Sample** tab is where you shape it: trim it, chop it into pieces across the pads, and decide how each pad plays.

The Sample tab always works on one pad at a time. **Hit a pad** to pick it; its name and sound are at the top left of the page. The tab has a page for each job, along the top: **Wave**, **Warp**, **Chop**, **Play**, **Grain**, **FX** and **Rec**. Beside them, two buttons work from any page: **Send to pad** and **Chop to pads** (see [below](#send-to-pad-and-chop-to-pads)).

Which sound is on which pad is the job of the **Pads** tab, next to Play: it shows the whole bank with the browser beside it.

> [!NOTE]
> Recording a new sample from the inputs or from MPFree's own sound onto a pad is coming to the Sample tab soon. Until then, loops recorded with the looper and the sounds in your kits are what you sample from.

## The Pads tab: sounds onto pads

The pads are the heart of an MPC, so they have a tab of their own, next to Play. On the left are the 16 pads of the bank, laid out as on the MPC (pad 1 at the bottom left), each with its name and how it plays. Tap a pad to pick it, and tap it again to hear it; a pad you hit lights up. Under the pads, **Copy**, **Paste** and **Clear** move pads around, and **Map to keys** turns the bank into a keyboard instrument (see [Playing a kit on keys](#playing-a-kit-on-keys)).

The switch at the top right shows **Browse** or **Pad** beside the pads.

{{shot: pads-browse | The Pads tab's Browse view: the bank on the left, the browser beside it in Samples / Drums, and Kick 01 picked, with its waveform and buttons.}}

**Browse** finds sounds on the stick. Pick a place on the left (**Samples**, **Recordings**, **Recent** or **Favourites**), open folders, and pick a sound to see its length, format and waveform. Then:

- **Load** puts it on the pad picked; **Load to pad** puts it on the next pad you hit.
- **Audition**, or **PLAY**, lets you hear it first, at the level Q-Link 3 sets.
- **Favourite** stars it, so it is in Favourites next time.

The data wheel, its push, **–** and **+** move through the browser too; see [Buttons and controls](../reference/controls.md#pads-tab). Browse needs the test stick's data partition; without it, the view says so and the Pad view still works.

{{shot: pads-pad | The Pads tab's Pad view: the picked pad's settings in the middle, and the sounds in the kit on the right.}}

**Pad** sets up the pad picked: its **sound**, how it plays (**Mode**: **One shot** plays to the end, **Gate** only while you hold the pad, **Toggle** starts and stops with each hit, **Loop** repeats its loop region, **Granular** plays it as grains), how many notes can ring at once (**Polyphony**), a **mute group** (pads in the same group cut each other off, like an open and a closed hi-hat), and its **level**, **pan** and **tune**. Q-Links 1 to 4 turn level, pan, tune and voices. **Put on** places the sound picked on the right onto the pad.

## Send to pad and Chop to pads

The two buttons at the top right of every Sample page:

- **Send to pad** copies the pad you are editing, its sound and every edit, onto the next pad you hit. The button says **Hit a pad** while it waits; press it again to call it off. One UNDO takes the copy back.
- **Chop to pads** opens the Chop page, so you can check the cuts first. Press it again there (or the Chop page's own button) to put the slices on the pads.

Both buttons are faint while the pad picked has no sound.

## Wave: trim and loop

{{shot: sample | The Wave page: the whole sample at the top, the zoomed region below, and the region's settings on the right. Send to pad and Chop to pads are at the top right.}}

The **Wave** page shows the sample. Set where it **starts** and **ends**, and where its **loop** starts and ends, with the four Q-Links. Hold **SHIFT** and press **+** to zoom in, **SHIFT + –** to zoom out (the Warp and Chop pages too). Tap the wave to hear it. On the right, set the region's **gain**, **tune**, **crossfade** (smooths a loop's seam), **warp** (play in time with the tempo) and **reverse**.

**Typing a number.** A value with a dotted line under it, such as **Warp tempo**, can be typed: tap the value and a number pad opens. Tap the number and **Enter**; the pad shows the range it takes and the value now, and says so if the number is out of range. **Cancel** leaves the value as it was. The data wheel and **–** / **+** step the number too, and pushing the wheel is Enter. **UNDO** takes a typed value back.

## Chop: slice across the pads

{{shot: sample-chop | The Chop page: the break cut into 15 slices at its hits, ready to put on pads A2 to A16.}}

**Chopping** cuts a sample into slices, one per pad, so you can replay a drum break in a new order or pick out single hits.

1. Pick a **Method**: **Transient** cuts at each hit, **Beat** on the beats, **Grid** into equal pieces, **Silence** at the gaps, **Manual** leaves the cuts to you, and **Live** cuts where you play (below).
2. Turn **Sensitivity** up for more slices, down for fewer.
3. Tap a slice to hear it. **Split** and **Merge** add or remove a cut by hand; the Q-Links move the picked slice's start.
4. Tap **Chop to** to put the slices on the pads, starting at the first pad you chose.

### Live: chop by playing

{{shot: sample-chop-live | Live chop: the break playing, with a cut dropped at each pad hit so far.}}

**Live** chops the way you hear it: the sound plays, and every pad you hit drops a cut at that moment.

1. Pick **Live** as the **Method**. The **Split** button becomes **Start**.
2. Tap **Start**. The sound plays from its start, and any cuts already there are cleared.
3. Hit any pad in time with the sound wherever a slice should begin. While live chop runs, the pads drop cuts instead of playing their own sounds.
4. The take ends when the sound ends, or when you tap **Stop**. Then tap **Chop to** as usual, or **Start** again for a new take.

**Pull to hits** (where **Sensitivity** is for the other methods) moves each cut onto the nearest drum hit within that many milliseconds, so a slightly early or late pad still cuts right on the hit; **Off** keeps each cut where you played it. With **Snap** on **Zero crossing**, a cut with no hit near it moves to the nearest point where the wave crosses zero, so its slice starts without a click. Nothing reaches the pads until you tap **Chop to**, so a take you don't like costs nothing: tap **Start** again.

> [!TIP]
> Live chop is quickest with a sound you know well: hit the pads where you would clap along, then fix any stray cut with **Split** and **Merge**.

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

**FX**, in the Sample tab's row of pages, opens the **chain editor** (see [Mixing and effects](mixing-and-effects.md#the-chain-editor)) on the pad you are editing, in the Sample tab's place. **Back** comes back to the Sample tab.

- **The pad's own effects.** The editor opens on the pad's own channel, so an effect added there plays on that pad alone. Add **Perform FX** for stutters, beat repeats, tape stops and reverse on just that pad, in time with the tempo. Each of its 16 slots (**Slot 1** to **Slot 16**) is a page in the editor, with its effect, its length and a **Hold** switch: Hold on plays it, off lets it go.
- **The whole kit's effects.** The switch at the top goes to **Drums**, the track every pad plays into, so an effect there plays on the whole kit, like a bus effect on the SP-404. To make it a one-knob effect, put its knobs on a macro (see [Macros](mixing-and-effects.md#macros)), and that macro's Q-Link plays them all at once.

The effects belong to the pad's place in the kit, not to its sound: put another sound on the pad and it plays through the same effects. Only bank A's 16 pads have a channel of their own for now. **FX** on a pad past them says so in the log.

## Auto-sample: a synth or your gear onto the pads

**Auto-sample** plays an instrument note by note, records every note and makes a playable instrument of it on the kit's pads: one pad a note, each covering the keys around it, with velocity layers and round robins if you ask for them. Open it with the **Auto-sample** button on the Sample tab's **Rec** page, set the notes and layers in the list, and tap **Start**. The bar says how many takes it makes, how long it takes and how much memory, before anything starts. **Cancel** stops and keeps the notes done, and one UNDO takes the whole instrument off.

The **Source** row says what plays:

- **The Synth tab's patch.** MPFree plays a copy of it, much faster than real time, so a whole instrument takes seconds.
- **External gear**, on a MIDI channel from 1 to 16. MPFree plays your synth through **MIDI OUT** and records it from the line input, in real time. Connect the synth's audio out to the line input, set it to the channel you picked and turn the line input on. MPFree first plays the middle note twice to hear how loud the synth is and how late it answers, so no note starts late or cut off. If the line input stops or you switch the Rec page to another source while it runs, it stops, lets go of the note and keeps the notes done.

> [!NOTE]
> **External gear** appears in the Source row once MIDI OUT is set up for it, which comes with the MIDI setup (see [Playing other gear](midi-and-cv.md)).

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

> [!MPC]
> The MPC's Sampler, Sample Edit and Program Edit modes are all on one tab in MPFree, always on the pad you last hit. **SAMPLER** opens the Sample tab, **SAMPLE EDIT** its Wave page, and **PROG EDIT** the Pads tab's Pad view (in the next build). **UNDO** takes back any edit on the Sample and Pads tabs.
