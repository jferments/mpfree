<!--
status: changing
covers: rust/mpfree-sampler rust/mpfree-sampler-ui/src/pads_tab.rs rust/mpfree-sampler-ui/src/rec_tab.rs rust/mpfree-sampler-ui/src/wave_tab.rs rust/mpfree-sampler-ui/src/warp_tab.rs rust/mpfree-sampler-ui/src/chop_tab.rs rust/mpfree-sampler-ui/src/auto_tab.rs rust/mpfree-app/src/sample_page.rs rust/mpfree-app/src/sampler.rs
checked: 2026-11-14 b76c3c7
-->

# Sampling

**Sampling** means taking a recording (a drum break, a chord from a record, your own voice) and playing it from the pads. This chapter covers getting sounds onto pads: loading them, recording them, trimming them, fitting them to the tempo and chopping them into slices. [Shaping how a pad plays](shaping-a-pad.md) then covers what each pad does when you hit it.

Two tabs share the work:

- The **Pads** tab, next to Play, shows the whole bank of 16 pads with the browser beside it: which sound is on which pad.
- The **Sample** tab works on one pad at a time. **Hit a pad** to pick it; its name and sound are at the top left of the page. Along the top are a page for each job: **Wave**, **Warp**, **Chop**, **Play**, **Grain**, **FX** and **Rec**. Beside them, **Send to pad** and **Chop to pads** work from any page (see [below](#send-to-pad-and-chop-to-pads)).

## The Pads tab: sounds onto pads

On the left of the Pads tab are the 16 pads of the bank, laid out as on the MPC (pad 1 at the bottom left), each with its name and how it plays. Tap a pad to pick it, and tap it again to hear it; a pad you hit lights up. Under the pads, **Copy**, **Paste** and **Clear** move pads around, and **Map to keys** turns the bank into a keyboard instrument (see [Playing a kit on keys](shaping-a-pad.md#playing-a-kit-on-keys)).

The switch at the top right shows **Browse** or **Pad** beside the pads.

{{shot: pads-browse | The Pads tab's Browse view: the bank on the left, the browser beside it in Samples / Drums, and Kick 01 picked, with its waveform and buttons.}}

**Browse** finds sounds on the stick. Pick a place on the left (**Samples**, **Recordings**, **Recent** or **Favourites**), open folders, and pick a sound to see its length, format and waveform. Then:

- **Load** puts it on the pad picked; **Load to pad** puts it on the next pad you hit.
- **Audition**, or **PLAY**, lets you hear it first, at the level Q-Link 3 sets.
- **Favourite** stars it, so it is in Favourites next time.

The data wheel, its push, **–** and **+** move through the browser too; see [Buttons and controls](../reference/controls.md#pads-tab). Browse needs the data partition on MPFree's stick; without it, the view says so and the Pad view still works.

**When memory runs short.** The MPC has 2 GB of memory, and running out would stop the sound, so MPFree always keeps 256 MB free. A sound that would leave less isn't loaded, and the hint line says so with how much it needed. The same goes for the sounds of a project you open (it opens without the ones that don't fit, and the Console lists them), for Auto-sample (it doesn't start), and for a warped pad's stretched copy (the pad speeds up or slows down with the tempo, like a record, instead of stretching). To make room, open a smaller project or a new one, or lower **Loop memory** or **Audio track memory** in [Prefs](../reference/prefs.md) for the next start.

{{shot: pads-pad | The Pads tab's Pad view: the picked pad's settings in the middle, and the sounds in the kit on the right.}}

**Pad** sets up the pad picked:

- its **sound**;
- how it plays (**Mode**): **One shot** plays to the end, **Gate** only while you hold the pad, **Toggle** starts and stops with each hit, **Loop** repeats its loop region, and **Granular** plays it as grains;
- how many notes can ring at once (**Polyphony**);
- a **mute group**: pads in the same group cut each other off, like an open and a closed hi-hat;
- its **level**, **pan** and **tune**;
- **Effects**: the effects on the pad's own mixer channel (**+ FX** when it has none). Tap it, or push the wheel on it, to open the pad's chain in the Mix tab, and **‹** brings you back (see [Effects](effects.md#adding-an-effect-the-chain-editor)). Only the first 16 pads have a channel of their own; the others show a dash.

Q-Links 1 to 4 turn level, pan, tune and voices. **Put on** places the sound picked on the right onto the pad, the whole of it; the hint line says when the pad plays all of it already.

## Recording a sample

The **Rec** page records onto a pad: from the INPUTS on the back of the MPC, or from MPFree's own sound (resampling).

{{shot: sample-rec | The Rec page recording the line input onto pad A6: a take that started on the bar line is two bars in, and the blue span is what Skip back would keep.}}

1. Pick what to record in the **Source** row. **Line in** is the INPUTS; turn the line input on first (**Prefs > Line input**, or the Looper tab's options), and set **REC VOL** so the meters stay out of the red. The other sources are MPFree's own sound: **Master** (everything you hear), a track (**Drums**, **Synth**, **Loop**), the delay's return, or one pad's channel.
2. Set how the take starts and ends (below), or leave them: by default it starts when you press **Record** and ends when you press **Stop**.
3. Tap **Record**, play, and tap **Stop**.

The take goes onto the next pad with no sound, starting from the pad picked, and the Sample tab moves to that pad so you can trim it straight away. The title says which pad it will land on ("Record onto pad A6"). Set **Onto** to **This pad** to replace the picked pad's sound instead. One **UNDO** takes a take off its pad again.

The picture shows the source's level on two meters, and the last 30 seconds of it scrolling past with the bar lines, so you can see what came in.

### Skip back: keep what you just played

The Rec page is always listening while its source is on. **Skip back** keeps what has already come in, without your having pressed Record: the last 10 seconds to begin with, or whatever the **Skip back** row says (5 seconds to a minute, or 1 to 8 whole bars cut on the bar lines). The blue span on the picture shows what it would keep. It is the sampler's version of [Capture](clips.md#capture-keep-what-you-just-played): play something good, then keep it.

### How a take starts and ends

The Record list on the right sets up the take. Pick a row with the data wheel or a tap, and change it with **–** and **+** or the arrows.

| Row | What it does |
|---|---|
| **Source** | What it records (above). |
| **Channels** | **Left + right**, **Left**, **Right** or **Mono mix** (both mixed into one). |
| **Start** | **Now**: when you tap Record. **Threshold**: when the sound gets louder than the Threshold row, for catching a hit without missing its start. **Next bar**: on the next bar line, in time with the music. |
| **Threshold** | How loud the sound must be to start a take (and quiet, to end one on silence), from -60 to -6 dB. |
| **Pre-roll** | With Start on Threshold, how much from just before the sound to keep as well (0 to 500 ms), so the attack isn't cut. |
| **End** | **Stop**: when you tap Stop. **Silence**: when the sound has been under the threshold for the Silence time. **Length**: after a number of bars. |
| **Length** | With End on Length: 1 to 32 bars. |
| **Tail** | What a take of a Length does at its end. **Cut** ends on the bar line. **Ring out** keeps recording until the sound dies away (up to 20 seconds more), for a one-shot with its reverb. **Loop** makes a loop of exactly its bars, with the ringing folded onto its start, so it loops with no seam. |
| **Silence for** | How long the quiet must last (250 ms to 4 s) to end a take on silence, or a Ring out. |
| **Skip back** | How far Skip back reaches. |
| **Onto** | **Next empty** or **This pad** (above). |
| **Flatten velocity**, **Flatten hold** | How Flatten plays the pad (below). |

A take that starts on the bar line and lasts whole bars lands warped to the project's tempo, so it stays in time if you change the tempo later. A take can be up to three minutes long.

### Flatten: print a pad to a new sound

**Flatten** records the picked pad as it sounds now, with its layers, envelope, filter, warp and pan in one new sound. It plays the pad once at the **Flatten velocity**, lets go after the **Flatten hold**, and records until the sound has been silent for the Silence time. Onto the next empty pad, the pad channel's effects are printed in too; onto the pad itself they are left out, because the channel will play them again.

Use it to free up a busy pad, or to chop a sound you have already shaped.

> [!TIP]
> Resampling is the classic MPC trick: play the loop, the synth or the whole mix into a pad, then chop it and play it again. Pick **Master** as the Source and use **Skip back** after a jam you liked.

## Send to pad and Chop to pads

The two buttons at the top right of every Sample page:

- **Send to pad** copies the pad you are editing, its sound and every edit, onto the next pad you hit. The button says **Hit a pad** while it waits; press it again to call it off. One UNDO takes the copy back.
- **Chop to pads** opens the Chop page, so you can check the cuts first. Press it again there (or the Chop page's own button) to put the slices on the pads.

Both buttons are faint while the pad picked has no sound.

## Wave: trim and loop

{{shot: sample | The Wave page: the whole sample at the top, the zoomed region below, and the region's settings on the right. Send to pad and Chop to pads are at the top right.}}

The **Wave** page shows the sample. Set where it **starts** and **ends**, and where its **loop** starts and ends, with the four Q-Links. Hold **SHIFT** and press **+** to zoom in, **SHIFT + –** to zoom out (the Warp and Chop pages too). Tap the wave to hear it. On the right, set the region's **gain**, **tune**, **crossfade** (smooths a loop's seam), **warp** and **reverse**.

**Warp** plays the sound in time with the project's tempo: **Stretch** keeps its pitch, **Repitch** speeds it up or slows it down like a record. **Warp tempo** is the tempo it was played at.

## Warp: put the hits on the beat

{{shot: sample-warp | The Warp page: a break at 92 BPM with 14 hits pinned to the 1/16 grid, pin 4 picked.}}

The **Warp** page is for a warped sound whose hits don't sit quite on the beat, such as a loop played by hand. It draws the sound against the bars and beats of its warp tempo, with the hits MPFree finds marked along the bottom.

- **Tap near a hit** to pin it to the nearest grid line. A pin is a numbered flag; the sound between two pins stretches to fit.
- **Drag a pin's handle** to move it along the grid.
- **Pin hits** pins every hit near a grid line onto it, which tightens a loose loop in one go. **Remove pin** takes the picked pin off, and **Clear pins** all of them.
- The list on the right holds **Warp** and **Warp tempo** (as on the Wave page), the **Grid** (1/4 to 1/32), which **Hits** it finds (**Strongest**, **Most** or **All**), and the picked **Pin** and its **Pin beat**.
- The Q-Links pick a pin, nudge its beat, fine-tune the warp tempo and scroll. Hold **SHIFT** for finer steps.

Pins only work while the sound is stretched. With Warp on Repitch, the sound plays straight through.

**Typing a number.** A value with a dotted line under it, such as **Warp tempo**, can be typed: tap the value and a number pad opens. Tap the number and **Enter**; the pad shows the range it takes and the value now, and says so if the number is out of range. **Cancel** leaves the value as it was. The data wheel and **–** / **+** step the number too, and pushing the wheel is Enter. **UNDO** takes a typed value back.

## Chop: slice across the pads

{{shot: sample-chop | The Chop page: the break cut into 15 slices at its hits, ready to put on pads A2 to A16.}}

**Chopping** cuts a sample into slices, one per pad, so you can replay a drum break in a new order or pick out single hits.

1. Pick a **Method**: **Transient** cuts at each hit, **Beat** on the beats, **Grid** into equal pieces, **Silence** at the gaps, **Manual** leaves the cuts to you, and **Live** cuts where you play (below).
2. Turn **Sensitivity** up for more slices, down for fewer.
3. Tap a slice to hear it. **Split** and **Merge** add or remove a cut by hand; drag a cut's tab, or turn the Q-Links, to move it. The first marker and **END** are the pad's own start and end, so they stay put here: move them on the Wave page.
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
> **Not yet:** the Put on row doesn't offer **Files** in this build. Exporting is built, and arrives once the Sample tab is connected to the stick.

## Auto-sample: a synth or your gear onto the pads

{{shot: sample-auto | Auto-sample on the Synth tab's Poly Pluck: a note every 5 semitones from C2 to A5, in three velocity layers, onto pads A6 to A15.}}

**Auto-sample** plays an instrument note by note, records every note and makes a playable instrument of it on the kit's pads: one pad a note, each covering the keys around it, with velocity layers and round robins if you ask for them. Open it with the **Auto-sample** button on the **Rec** page. In the list, set the **Lowest note** and **Highest note**, how far apart the sampled notes are (**Every**), the velocity **Layers**, **Round robins**, how long each note is held (**Hold**) and how long its tail may ring (**Tail**). Then tap **Start**; **Rec** goes back to the Rec page. The bar says how many takes it makes, how long it takes and how much memory, before anything starts. **Cancel** stops and keeps the notes done, and one UNDO takes the whole instrument off.

The **Source** row says what plays:

- **The Synth tab's patch.** MPFree plays a copy of it, much faster than real time, so a whole instrument takes seconds.
- **External gear**, on a MIDI channel from 1 to 16. MPFree plays your synth through **MIDI OUT** and records it from the line input, in real time. Connect the synth's audio out to the line input, set it to the channel you picked and turn the line input on. MPFree first plays the middle note twice to hear how loud the synth is and how late it answers, so no note starts late or cut off. If the line input stops or you switch the Rec page to another source while it runs, it stops, lets go of the note and keeps the notes done.

> [!NOTE]
> **Not yet:** **External gear** needs MIDI OUT, which is still off in this build (see [Playing other gear](midi-and-cv.md)). Until then the Source row offers the Synth tab's patch only.

> [!MPC]
> The MPC's Sampler, Sample Edit and Program Edit modes are all on one tab in MPFree, always on the pad you last hit. **SAMPLER** opens the Sample tab, **SAMPLE EDIT** its Wave page, and **PROG EDIT** the Pads tab's Pad view. **UNDO** takes back any edit on the Sample and Pads tabs.
