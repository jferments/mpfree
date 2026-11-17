<!--
status: changing
covers: rust/mpfree-sampler rust/mpfree-sampler-ui/src/pads_tab.rs rust/mpfree-sampler-ui/src/rec_tab.rs rust/mpfree-sampler-ui/src/wave_tab.rs rust/mpfree-sampler-ui/src/warp_tab.rs rust/mpfree-sampler-ui/src/chop_tab.rs rust/mpfree-sampler-ui/src/auto_tab.rs rust/mpfree-app/src/sample_page.rs rust/mpfree-app/src/sampler.rs
checked: 2026-11-16 ececc7e
-->

# Sampling

**Sampling** means taking a recording (a drum break, a chord from a record, your own voice) and playing it from the pads. This chapter covers getting sounds onto pads: loading them, recording them, trimming them, fitting them to the tempo and chopping them into slices. [Shaping how a pad plays](shaping-a-pad.md) then covers what each pad does when you hit it.

Two tabs share the work:

- The **Pads** tab, next to Play, shows the whole bank of 16 pads with the browser beside it: which sound is on which pad.
- The **Sample** tab works on one pad at a time. **Hit a pad** to pick it; its name and sound are at the top left of the page. Along the top are a page for each job: **Wave**, **Warp**, **Chop**, **Play**, **Grain**, **FX** and **Rec**. Beside them, **Send to pad** and **Chop to pads** work from any page (see [below](#send-to-pad-and-chop-to-pads)). The four **Q-Link** cards are down the right-hand side, as on every other page: each says what its knob turns here, and you can drag a card up or down to turn it from the screen.

## The Pads tab: sounds onto pads

On the left of the Pads tab are the 16 pads of the bank, laid out as on the MPC (pad 1 at the bottom left), each with its name and how it plays. Tap a pad to pick it, and tap it again to hear it; a pad you hit lights up. Under the pads, **Copy**, **Paste** and **Clear** move pads around, and **Map to keys** turns the bank into a keyboard instrument (see [Playing a kit on keys](shaping-a-pad.md#playing-a-kit-on-keys)).

The switch at the top right shows **Browse** or **Pad** beside the pads.

{{shot: pads-browse | The Pads tab's Browse view: the bank on the left, the browser beside it in Samples / Drums, and Kick 01 picked, with its waveform and buttons.}}

**Browse** finds sounds and kits on the stick. Pick a place on the left (**Samples**, **Kits**, **Recordings**, **Recent** or **Favourites**). Tap a folder to open it, and tap a sound to pick it and see its length, format and waveform. Then:

- **Load** puts it on the pad picked (so does a second tap on it); **Load to pad** puts it on the next pad you hit.
- **Audition**, or **PLAY**, lets you hear it first, at the level Q-Link 3 sets.
- The **star** at the right end of each row adds it to Favourites; tap it again to take it out.

**16-bit sounds take half the memory.** A 16-bit WAV or AIFF, and anything you trim, slice or cut from one, is kept as 16 bits: about 10 MB a minute of stereo, half what a 24-bit or 32-bit float sound takes. It sounds exactly the same.

**Sounds at other rates.** MPFree plays at 44.1 kHz. A sound recorded at another rate (48 kHz from a video or a phone, 96 kHz from a studio, 22 kHz from an old sampler) is converted to 44.1 kHz as it loads, and the Console says so ("converted from 48 kHz to 44.1 kHz"). Nothing you can hear changes: it plays at its own pitch, and as lightly as a sound recorded at 44.1 kHz. Whatever its settings file places on it (trim, loop, fades, warp and slices) moves with it to the same moments, and the file itself stays as it was. Kits and projects convert their sounds as they open, too. When memory is short, a sound stays at its own rate; it still plays at its pitch, and the Console lists it.

**Whole kits.** A kit (green icon) loads all at once: pick it and tap it again, push the data wheel, or tap **Load**. It takes the place of the kit the pads play now, with every pad's sound and settings. To see the sounds inside a kit one by one, press **+**. **Save kit**, at the top of the **Kits** place, keeps the kit the pads play as a kit of its own, so you can load it in another project; save it before you load another kit over it if you want to keep your changes. MPFree puts its own drum kits in **Kits > MPFree** the first time it starts with your library.

**MPFree's free kits.** Next to MPFree's own kits, made by its drum synth, are three kits of real recorded drums: **Big Rusty Drums** (a big acoustic kit) and **Swirly Drums** (a second acoustic kit, with cowbell, splash and china), both recorded by Karoryfer Samples, and **VCSL Percussion** (claves, cajon, tambourine, shaker, claps, bongos, congas, cowbell, agogo, triangle and more), from Versilian Studios' community sample library. Their makers gave the recordings to the public domain (CC0), so anything you make with them is yours, with no credit or fee owed. MPFree took one hit per pad from each library and brought every hit to about the same loudness, so set each pad's level to taste. Each hit's source is listed in the file `free-kits-credits.txt` that comes with the USB creator's download.

The data wheel, its push, **–** and **+** move through the browser too; see [Buttons and controls](../reference/controls.md#pads-tab). Browse needs the data partition on MPFree's stick; without it, the view says so and the Pad view still works.

**When memory runs short.** The MPC has 2 GB of memory, and running out would stop the sound, so MPFree always keeps 256 MB free. A sound that would leave less isn't loaded, and the hint line says so with how much it needed. The same goes for the sounds of a project you open (it opens without the ones that don't fit, and the Console lists them), for Auto-sample (it doesn't start), and for a warped pad's stretched copy (the pad speeds up or slows down with the tempo, like a record, instead of stretching). To make room, open a smaller project or a new one, or lower **Loop memory** or **Audio track memory** in [Prefs](../reference/prefs.md) for the next start.

{{shot: pads-pad | The Pads tab's Pad view: the picked pad's settings in the middle, and the sounds in the kit on the right.}}

**Pad** sets up the pad picked:

- its **sound**;
- how it plays (**Mode**): **One shot** plays to the end, **Gate** only while you hold the pad, **Toggle** starts and stops with each hit, **Loop** repeats its loop region, and **Granular** plays it as grains;
- how many notes can ring at once (**Polyphony**). Kit pads, and a sound you load onto an empty pad, start at 4: hit a pad again (or let Note Repeat do it) and its fifth hit stops its oldest, while other pads' sounds ring on. MPFree plays up to 64 sounds at once: plain drum hits (One shot, no filter, the envelope open, at the sound's own pitch) can use all 64, and sounds with a filter, an envelope, a loop, grains or a tune up to 24 of them. When too many sounds play at once, MPFree stops the oldest sound of the pad that has the most playing first;
- a **mute group**: pads in the same group cut each other off, like an open and a closed hi-hat;
- its **level**, **pan** and **tune**;
- **Effects**: the effects on the pad's own mixer channel (**+ FX** when it has none). Tap it, or push the wheel on it, to open the pad's chain in the Mix tab, and **‹** brings you back (see [Effects](effects.md#adding-an-effect-the-chain-editor)). Only the first 16 pads have a channel of their own; the others show a dash.

Q-Links 1 to 4 turn level, pan, tune and voices. **Put on** places the sound picked on the right onto the pad, the whole of it; the hint line says when the pad plays all of it already.

## A kit for each track

Each track plays a kit of its own, as each track plays a program on the MPC, and every track's kit sounds at the same time: a drum kit on track 1, chopped vocals on track 2 and one-shot stabs on track 3 all play together in the song.

- **The pads play the selected track's kit.** Select another track (on Arrange, or with the track chip) and the pads play its kit. A track's clips play its kit whichever track is selected.
- **To change a track's kit,** select the track and pick the kit in the kit chip's list at the top right, or with **SHIFT** + the data wheel on the Play page, or with **PAD BANK**. Only these change what a track plays: opening a page never does. The list's last row is the synth (see [Put a synth on the pads](synths.md#put-a-synth-on-the-pads)).
- **Every track starts on kit 1,** so a project saved before tracks had a kit each plays as it did.
- **In the mixer,** kit 1 plays through the 16 pad channels and the **Drums** strip, as before. Another kit plays into the strip of the first track that plays it (**Track 2**, say), with its own fader, effects and sends; tracks that play the same kit share it. That kit's pads have no channels of their own yet, so put its effects on the track's strip, and mute the track rather than a pad: on its track the Play page's pad speakers and **TRACK MUTE**'s pad mutes say so and mute nothing.
- **The project keeps** which kit each track plays.

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

## Naming a pad and its sound

{{shot: sample-name | The text keyboard: the pad's name with the cursor at its end, Pad and Sound at the top right, and Cancel, Space and Enter along the bottom.}}

The pad's name at the top left of every Sample page has a dotted line under it: tap it to type a new one. The keyboard opens with the name as it is, so **Delete** takes off the last letter and **Clear** the whole name. **Shift** gives one capital (or the symbol shown on the key); an empty name starts with one. **#+=** turns the letter keys into symbols (# & + = ( ) [ ] ! ? @ ~ and more) until you press it again (it says **ABC** then). Turn the data wheel, or press **–** / **+**, to move the cursor and fix a letter in the middle.

{{shot: sample-name-symbols | The keyboard with #+= on: the letter keys type symbols, and the key says ABC to turn them back.}}

**Pad** and **Sound** at the top right pick which name you are typing: the pad's, or the name of the sound it plays (a recording's "Take 3", say). A sound's name is the same on every pad that plays it, and the keyboard says how many that is. Tap **Enter** (or push the wheel) to keep both names; **Cancel** leaves them as they were. One **UNDO** takes both back. A name can have up to 32 characters, and can't be empty.

## Wave: trim and loop

{{shot: sample | The Wave page: the whole sample at the top, the zoomed region across the page, the region's knobs, buttons and switches under it in five sections, and the four Q-Links (Start, End, Loop start, Loop end) down the right. Send to pad and Chop to pads are at the top right.}}

The **Wave** page shows the sample across the screen. Set where it **starts** and **ends**, and where its **loop** starts and ends, with the four Q-Links, or rest a finger on a marker's tab and turn the data wheel: it moves a screen pixel a turn, or one frame at a time with **SHIFT** (next build; see [Hold and turn](../getting-started/the-screen.md#hold-and-turn)). Hold **SHIFT** and press **+** to zoom in, **SHIFT + –** to zoom out (the Warp and Chop pages too); the card beside the strip at the top says how far in you are. Tap the wave to hear it.

Under the wave, the region's settings are laid out like an instrument's panel, in five sections:

- **Mode:** how a hit plays (**One shot**, **Gate**, **Toggle**, **Loop** or **Granular**), the **Loop** (**Off**, **Forward** or **Ping-pong**) and its **Crossfade**, which smooths a forward loop's seam. Tap a button to pick it.
- **Level:** the **Gain** and **Tune** knobs, and **Normalize to 0 dB**.
- **Fades:** **Fade in** and **Fade out**.
- **Warp:** **Off**, **Stretch** or **Repitch**, and the **Warp tempo** (tap the number to type one).
- **Options:** **Reverse**, how markers **Snap** (off, to a zero crossing, or to 16ths) and which **Channels** play.

Drag a knob up to turn it up and down to turn it down; one drag is one **UNDO**. Tap ‹ › or − + to step a setting. Touching a control also gives it the wheel and **–** / **+**, and an outline shows which one has it; turn the wheel to pick the next control along. Or rest a finger on any control with a value and turn the wheel: it turns that control until you lift (see [Hold and turn](../getting-started/the-screen.md#hold-and-turn)). A grey control has nothing to do yet, such as Crossfade without a forward loop.

**Normalize to 0 dB** (under Gain and Tune) turns the gain so the loudest moment between Start and End plays at full level, 0 dB, and never over. Tap it, or push the wheel on it. It only sets the gain, so after moving Start or End you can tap it again for the new part; **UNDO** takes it back. A silent part is left as it is.

MPFree finds each sound's hits, tempo and note (for Chop, Warp, the 16ths snap and Map to keys) while you work, never holding up the screen. A long sound shows as a flat line for a moment, then its wave, then its hits and tempo a little later: a minute-long sound takes a second or two on the MPC. The pages fill in as soon as each is found, and a sound already looked at is ready at once. A saved project keeps what was found beside each sound, so when you open it again its sounds are ready at once too.

**Warp** plays the sound in time with the project's tempo: **Stretch** keeps its pitch, **Repitch** speeds it up or slows it down like a record. **Warp tempo** is the tempo it was played at.

**Edits you hear while it plays.** Hold a looping pad (or tap the wave in Loop mode, so it keeps sounding) and move its loop: the loop that is playing moves too. Moved behind the sound, it jumps into the new loop without a click; moved ahead, you hear it when the sound gets there. The pad's level, pan, tune, gain, filter cutoff and resonance, and a grain cloud's settings change on a sound still playing too, smoothly. The start, end, fades, reverse, mode, envelope and filter type, and a new sound on the pad, are heard from the next hit.

## Warp: put the hits on the beat

{{shot: sample-warp | The Warp page: a break at 92 BPM with 14 hits pinned to the 1/16 grid, pin 4 picked.}}

The **Warp** page is for a warped sound whose hits don't sit quite on the beat, such as a loop played by hand. It draws the sound against the bars and beats of its warp tempo, with the hits MPFree finds marked along the bottom.

- **Tap near a hit** to pin it to the nearest grid line. A pin is a numbered flag; the sound between two pins stretches to fit.
- **Drag a pin's handle** to move it along the grid.
- **Pin hits** pins every hit near a grid line onto it, which tightens a loose loop in one go. **Remove pin** takes the picked pin off, and **Clear pins** all of them.
- The list on the right holds **Warp** and **Warp tempo** (as on the Wave page), the **Grid** (1/4 to 1/32), which **Hits** it finds (**Strongest**, **Most** or **All**), and the picked **Pin** and its **Pin beat**.
- **Process** makes a new sound from the region. With Warp on, the new sound is at the project's tempo, its pinned hits right on their beats, so it plays without stretching. **Shift** first moves it up or down, in semitones (–24 to +24), keeping its length and its hits sharp; with Warp off, Shift alone is what changes. Tap **New sound**: it is made in the background while you work, the button shows how far it has got, and tapping it again stops it. The new sound takes the region's place; **UNDO** brings the old one back.
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

**Your chop stays with the sound.** Once you change the cuts (a method, a split, a cut moved, a live take), MPFree keeps them for that sound: pick another pad and come back, or open the project tomorrow, and the Chop page shows your slices as you left them, not a fresh chop. Keeping them isn't an edit, so **UNDO** leaves them be; they go on pads only when you tap **Chop to**.

### Live: chop by playing

{{shot: sample-chop-live | Live chop: the break playing, with a cut dropped at each pad hit so far.}}

**Live** chops the way you hear it: the sound plays, and every pad you hit drops a cut at that moment.

1. Pick **Live** as the **Method**. The **Split** button becomes **Start**.
2. Tap **Start**. The sound plays from its start, and any cuts already there are cleared.
3. Hit any pad in time with the sound wherever a slice should begin. While live chop runs, the pads drop cuts instead of playing their own sounds.
4. The take ends when the sound ends, or when you tap **Stop**. Then tap **Chop to** as usual, or **Start** again for a new take.

**Pull to hits** (where **Sensitivity** is for the other methods) moves each cut onto the nearest drum hit within that many milliseconds, so a slightly early or late pad still cuts right on the hit; **Off** keeps each cut where you played it. With **Snap** on **Zero crossing**, a cut with no hit near it moves to the nearest point where the wave crosses zero, so its slice starts without a click. Nothing reaches the pads until you tap **Chop to**, so a take you don't like costs nothing: tap **Start** again (the page keeps your last take for the sound).

> [!TIP]
> Live chop is quickest with a sound you know well: hit the pads where you would clap along, then fix any stray cut with **Split** and **Merge**.

### Exporting a pad as files

With **Put on** at **Files**, the button reads **Export** (**Export 15 slices**, or **Export the pad** when there are no cuts) and, instead of filling pads, it saves the pad as files in a folder of its own under **Samples › Exports** in the browser:

- **Break 92.wav**: the pad's whole sound. Its loop and slice points are inside it too, where most samplers and audio editors find them.
- **Break 92.mpfree**: the pad's settings, a small text file beside the sound: its trim, loop, warp, envelope, filter, layers and slices.
- **Break 92 01.wav**, **Break 92 02.wav** and so on: each slice as a file of its own, to use anywhere.

Load **Break 92.wav** back, on this MPC or another, and the pad comes back exactly as you exported it, slices and all, as long as **Break 92.mpfree** is beside it. Without it, the sound loads on its own. Export the same pad again and it goes into **Break 92 2**, so nothing is written over.

**Files** is offered when MPFree has its library on the stick; without one there is nowhere to put them.

## Auto-sample: a synth or your gear onto the pads

{{shot: sample-auto | Auto-sample on the Synth tab's Poly Pluck: a note every 5 semitones from C2 to A5, in three velocity layers, onto pads A6 to A15.}}

**Auto-sample** plays an instrument note by note, records every note and makes a playable instrument of it on the kit's pads: one pad a note, each covering the keys around it, with velocity layers and round robins if you ask for them. Open it with the **Auto-sample** button on the **Rec** page. In the list, set the **Lowest note** and **Highest note**, how far apart the sampled notes are (**Every**), the velocity **Layers**, **Round robins**, how long each note is held (**Hold**) and how long its tail may ring (**Tail**). Then tap **Start**; **Rec** goes back to the Rec page. The bar says how many takes it makes, how long it takes and how much memory, before anything starts. **Cancel** stops and keeps the notes done, and one UNDO takes the whole instrument off.

The **Source** row says what plays:

- **The Synth tab's patch.** MPFree plays a copy of it, much faster than real time, so a whole instrument takes seconds.
- **External gear**, on a MIDI channel from 1 to 16. MPFree plays your synth through **MIDI OUT** and records it from the line input, in real time. Connect the synth's audio out to the line input, set it to the channel you picked and turn the line input on. MPFree first plays the middle note twice to hear how loud the synth is and how late it answers, so no note starts late or cut off. If the line input stops or you switch the Rec page to another source while it runs, it stops, lets go of the note and keeps the notes done.

The notes go out of the 5-pin **MIDI OUT** jack. MIDI out is built but not yet checked on an MPC One, so it only sends while **Send MIDI and CV out** is on in [Prefs](../reference/prefs.md) (see [Playing other gear](midi-and-cv.md)). Until then the Source row offers the Synth tab's patch only, and with the gear picked the bar says there is no MIDI out for it. Every note it plays is let go when it ends or you cancel it, and the MIDI Monitor shows them as sent by the auto-sample.

> [!MPC]
> The MPC's Sampler, Sample Edit and Program Edit modes are all on one tab in MPFree, always on the pad you last hit. **SAMPLER** opens the Sample tab, **SAMPLE EDIT** its Wave page, and **PROG EDIT** the Pads tab's Pad view. **UNDO** takes back any edit on the Sample and Pads tabs.
