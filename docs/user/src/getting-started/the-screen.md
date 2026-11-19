<!--
status: changing
covers: rust/mpfree-ui/src/widgets.rs rust/mpfree-ui/src/qlink.rs rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/gui.rs
checked: 2026-11-19 f0b39161
-->

# The screen

Every page in MPFree is laid out the same way, so once you know one, you know them all. From top to bottom: the **top bar** with the tabs, a **transport bar** on the pages that show time, the page itself, and the **hint line** along the bottom. Down the right edge are the **master bar** and the **Q-Link column**.

{{shot: play | The Play page. The tabs are at the top left, the chips and the folder at the top right, the master bar and the Q-Link column down the right edge, and the hint line along the bottom.}}

## The top bar

**The tabs** on the left are MPFree's main pages. Tap one to go there. The tab you are on is lit.

| Tab | What it is for | Chapter |
|---|---|---|
| **Play** | The home page: the loops, the pads as rows of steps, the throws and the XY pad. | [Playing live](../making-music/playing-live.md) |
| **Sequencer** | The armed track's clip, as steps (**Steps**) or as notes (**Notes**). **STEP SEQ** opens it. | [The Sequencer](../making-music/sequencer.md), [Editing notes](../making-music/editing-notes.md) |
| **Looper** | Up to eight loops side by side, and the picked loop's layers. | [Recording loops](../making-music/looping.md) |
| **Sample** | Recording, trimming, chopping and shaping the sound on one pad. | [Sampling](../making-music/sampling.md) |
| **Synth** | MPFree's synths and their sounds, or an MPC plugin's own screen. | [Synths](../making-music/synths.md) |
| **Arrange** | Clips, scenes and the song: recording clips, keeping takes and launching them. | [Clips and patterns](../making-music/clips.md), [Arranging a song](../making-music/arranging.md) |
| **Pads** | The kit's pads beside the picked pad's settings: put a sound on each pad and decide how it plays. | [The Pads tab](../making-music/pads.md) |
| **Mix** | The mixer, every sound's effects and the I/O view. | [Mixing](../making-music/mixing-and-effects.md), [Effects](../making-music/effects.md) |

A ninth tab, **Dev**, shows only when **Show developer tools** is on in [Prefs](../reference/prefs.md). It holds the tools for people building MPFree (the MIDI log, the performance pages, the console) and you can ignore it.

**The chips** on the right show what is going on. Most only show while they have something to say:

| Chip | Shows | A tap |
|---|---|---|
| **MAKE** or **PERFORM** | Which way MPFree answers your hands (see [Make and Perform](../making-music/playing-live.md#make-and-perform)). Always there, at the left of the chips. | Switches, as **SHIFT + MAIN** does. |
| **Full level**, **Half level** or **Level 92** | Every pad hit plays at full (or half) level, however hard you play, or at the level you set by holding **FULL LEVEL** and hitting a pad. | Turns that level off, as pressing **FULL LEVEL** again would. |
| **Pads: steps · Kick** | **STEP SEQ** has made the pads the steps of a sound (with a synth, the synth and the note new steps play: **Pads: steps · Poly · C3 + G3**). | Gives the pads back, as **STEP SEQ** again would. See [The pads as steps](../making-music/sequencer.md#the-pads-as-steps-step-seq). |
| **Pads: Perform B** | The pads are a page of the Perform set (throws, mutes, scenes). | Gives the pads back. See [The Perform set](../making-music/looping.md#more-pages-the-perform-set). |
| **16 Levels · tune** | **16 LEVEL** has spread one sound over the pads. | Says how to turn it off. See [16 Levels](../making-music/playing-live.md#16-levels). |
| **Controller → Juno-X · ch 3 · A** | Controller mode is on: the pads and Q-Links play your outside gear, not the kit. | Turns controller mode off. See [Controller mode](../making-music/midi-and-cv.md#controller-mode-the-pads-and-q-links-play-your-gear). |
| **Arp · Up · 1/16** | The arpeggiator, Note Repeat's latch or a pattern is on. | Opens the Arp panel. See [Note Repeat, Latch and the arpeggiator](../making-music/note-repeat-and-arp.md). |
| **120.0 from MIDI In** | The tempo follows a clock from another device (with a green dot). | Opens the I/O view at Sync. |
| **READY → Track 1** | On a page with no transport bar: REC is ready or recording, and what it records. | Says what it shows. |
| **CPU 21 %** and **RAM 40 %** | How much of the processor the sound uses and how much memory is in use: green, then amber over 70 % and red over 85 %. They keep their width as the numbers change, so the chips beside them stay put. | Says what it shows. |
| **No session** | Only when something is wrong with the link to the pads and buttons. | Says what it shows. |

When memory runs so short that new sounds can't load, the hint line says so (see [The Pads tab](../making-music/pads.md)).

**The folder** at the far right of the bar opens the browser over the page, as **BROWSE** does, and is lit while the browser shows. Tap it again (or press BROWSE, or tap a tab) to close it. See [The browser](../making-music/saving.md#the-browser).

## The tabs at a glance

Each tab is drawn here as MPFree draws it, with sample music in it. The chapter for each one has more pictures.

{{shot: sequencer-steps | The Sequencer tab, on Steps: the armed track's pattern with two lock lanes, your timing under it, and the step inspector for the picked step at the bottom.}}

{{shot: looper | The Looper tab: the eight loops along the top (three playing, two off, one recording, two empty), and the picked loop's three layers underneath with its overdub, speed, length and history controls.}}

{{shot: sample | The Sample tab: the pad bar at the top, the recorder row, the wave with its markers and tools, and the pad's mode, gain, options and sound underneath.}}

{{shot: synth | The Synth tab: the synth and its presets on the left, the sound's pages along the top, and the knobs of the Osc page. The coloured tags (Q1 to Q4) show which knobs the Q-Links turn.}}

{{shot: clips | The Arrange tab's Clips grid mid-jam: eight tracks across the top (kits, synths and a loop track), scenes down the side, clips playing, waiting for the bar and recording, empty slots to fill, the Record settings on the right, and the scene morph along the bottom.}}

{{shot: pads-pad | The Pads tab: the tracks with kits, the kit's 16 pads in the picked bank (each labelled with what it is: a sample, a loop, a synth voice or MIDI out), the banks along the top with a dot for each pad in use, the picked pad's settings in the middle, and the kit's sounds on the right.}}

{{shot: mixer | The Mix tab on Faders: a strip for each track, with its effects, sends, pan, fader, mute, solo and output, and the master strip on the right.}}

## The transport bar

On the Play page and the Sequencer tab, a bar under the tabs shows time:

- **Stopped**, **Playing** or **Recording**.
- **The position**: bar, beat and step (**9.2.03** is bar 9, beat 2, step 3).
- **The tempo** in BPM. Tap it in time to set the tempo, as with **TAP TEMPO**, or hold a finger on it to type the tempo on the number pad (holding **TAP TEMPO** and letting go does the same).
- **The REC chip**: what **REC** will record, such as **→ Track 1 · Pattern 1**. Tap it for the list of everything that can record (see [What REC records](../making-music/sequencer.md#what-rec-records)).
- **Dots** that light up with each bar and beat, with a meter of the beat under them.
- At the right end: on the Sequencer's Steps, the view switch **Steps** and **Notes**. On Play, **PADS PLAY**.

**PADS PLAY** says what the pads play: the selected track, its kit and the pad bank, such as **Track 1 · Classic House · A** (or the synth, such as **Keys · Poly · C3 to D#4**). Tap it for the list of kits. While controller mode is on, the pads play your outside gear, so it says what the **Controller** chip says.

The Arrange tab, and the Sequencer's Notes view, have a transport bar of their own, with **Stop**, **Play** and **Rec** buttons, the record mode, the position, the tempo, the time signature and the launch line (see [Clips and patterns](../making-music/clips.md)).

## The list of kits

The list of kits opens over the page from **PADS PLAY** on Play or **Kit** on the Pads tab. Tap a kit to load it on the pads; the list closes and you are back where you were. The synth comes after the kits, and the last row, **New kit**, puts an empty kit on the selected track (while there is an empty one left). **Back** or any tab closes the list and keeps the kit that is playing. You can also hold **SHIFT** and turn the data wheel on the Play page to step through the kits without opening the list.

{{shot: kits | The list of kits, opened from PADS PLAY. Tap a kit to load it on the pads; Back closes the list.}}

## The master bar and the Q-Link column

At the right edge, the **Q-Link column** shows four cards, one for each Q-Link knob beside the screen, in the same order (knob 1 at the top). Each card shows the knob's number and what it controls, over a ring with its value in it. Turn the knob, or drag up and down on its card. The knobs are touch-sensitive: rest a finger on one and its card lights up, so you can see what a knob does before you turn it. What the cards control changes with the page; each chapter has a table.

Next to it, the **master bar** shows how loud MPFree's output is: two meters (left and right) that hold their peaks, the level as a number under them, and the master fader's handle across them. Drag the handle to set the overall level. It starts at -6 dB, the level the browser auditions sounds at, so a sound plays as loud on the pads as it did when you heard it in the browser. **LIM** lights under the meters, with how much it is holding back, when the master's safety limiter is stopping the output from distorting (see [The master and Master safety](../making-music/mixing-and-effects.md#the-master-and-master-safety)).

## The hint line

The line along the bottom of the screen always says what the buttons and taps do on the page you are on. If you're not sure what to press, look there first.

When something you asked for can't happen, or does nothing, the hint line says why in amber for a few seconds, then goes back to the hint. For example: "The chain is full", or why a sample edit was refused.

The same goes for a faint button or card: tap it and the hint line says why it is faint and what to do first, such as "Copy a pad first" or "The filter is off: pick one in the Filter row". A setting that is already as low or as high as it goes says so too, such as "Count-in is off already". A tab, row or card that is already lit has nothing new to show, so a tap on it changes nothing.

## Things that open over the page

Some things are not tabs. They open over the page you are on, and close the same way they opened:

- **Options:** press **MENU** for the current page's settings, such as the repeat rate or the tempo on the Play page. **MENU** again closes them.
- **Prefs:** press **SHIFT + MENU** for MPFree's settings. See [Prefs](../reference/prefs.md).
- **The browser:** press **BROWSE**, or tap the folder at the top right. See [The browser](../making-music/saving.md#the-browser).
- **The list of kits:** tap **PADS PLAY** on Play's transport bar, or **Kit** on the Pads tab.
- **The Arp panel:** hold **NOTE REPEAT** and press **MENU**, or tap the arp chip.
- **The undo history:** hold **UNDO** (see [Undo](#undo)).
- **The button guide:** see [Buttons and controls](../reference/controls.md).

{{shot: options | Play's options, opened with MENU. Turn the wheel or tap to pick a row; – and + change it. The panel on the right explains the row you picked.}}

## Lists of settings

Options, Prefs, the settings beside the Sample tab's wave and on the Pads tab, the Synth tab's tiles and Arrange's Record settings all work the same way:

1. **Pick a row:** turn the data wheel, or tap the row.
2. **Change it:** press **–** or **+**, or tap the arrows at either end of its value.
3. **Or turn it:** push the data wheel. The row's value is outlined and the wheel now changes it. Push again to set it, and the wheel moves between rows again.

The panel beside a list explains the row you picked. A list longer than its panel scrolls a row at a time as you pick, and a finger can drag it up or down; the thin bar at its right shows where you are. Holding **SHIFT** gives the wheel its other job on that page (the next pad, the next page, scrolling): the hint line says which.

## Typing a number or a name

A value with a **dotted line** under it can be typed:

- **A number** (the tempo, the Warp tool's tempo) opens the **number pad** over the page: the digits, the point, the minus where the value can go below zero, **Delete**, **Cancel** and **Enter**. It shows the range and the value now. A number out of range keeps the pad open and says the range in red. The wheel and **–** and **+** step the number too, and the wheel's push is Enter.
- **A name** (a pad, its sound, a USB MIDI device) opens the **keyboard**: the digits, the letters, **Shift** for one capital, **#+=** for symbols, **Space**, **Delete**, **Clear**, **Cancel** and **Enter**. A name can be up to 32 characters (24 for a MIDI device). Enter with the name left empty keeps the keyboard open and says so. The wheel and **–** and **+** move the cursor.

**Cancel**, or changing page, leaves the value as it was. A typed value is one **UNDO** step.

{{shot: sample-name | Naming a pad: the keyboard over the Sample tab, with the Pad and Sound switch at the top right.}}

## The wheel's outline

On the Play page, the Looper tab, the mixer and the FX chain editor, the data wheel takes you round the screen without touching it:

- **Tap a control** and an outline appears on it. **Turn the wheel** and the outline moves to the next control, left to right and top to bottom through that part of the screen, then on into the next part. On Play and the Looper tab you can also just turn the wheel: the outline starts on the first control.
- **Push the wheel** to press the control the outline is on, as a tap would. On a throw, the push holds it on until you push again.
- **On a value** (a fader, a pan, a send, a Q-Link card, the master level, an effect's knob), the push grabs it: the outline gets thicker and the wheel turns the value. Push again to let go, and the wheel moves the outline again. **–** and **+** step an outlined value without the push.
- **In the chain editor**, push on the picked effect to pick it up: the wheel (or **–** and **+**) moves it up or down the chain, and a push puts it down.
- Some values can't be turned with the wheel yet (the XY pad, the Looper tab's trims); the hint line says to drag them.
- On the mixer and the chain editor, the wheel keeps its old job (the selected strip's volume, the first knob) until you tap a control. Changing page, or tapping an empty spot, takes the outline away.
- While the list of effects is open, the wheel moves through the effects and its push opens presets and then adds (see [Adding an effect](../making-music/effects.md#adding-an-effect-the-chain-editor)).

## Hold and turn

On every page, **rest a finger on anything with a value and turn the data wheel**: the wheel turns that value for as long as your finger stays down, and **–** and **+** step it. It gets a thick outline, and the hint line says what it is and what it reads now. Lift your finger and the wheel goes back to its usual job on that page. There is no wait: the wheel takes the control as soon as your finger is down.

- **What it turns:** knobs, faders, pans and sends, sliders, Q-Link cards and macro cards, an effect's knobs, the Synth tab's knobs, choices and switches, the settings rows of the Sample and Pads tabs, Arrange and Prefs (the whole row, not just its arrows), and the Sample tab's markers. A marker moves a screen pixel a turn at the zoom showing, snapped as its Q-Link moves it. On buttons, pads, tabs and list rows that only pick something, the wheel keeps its usual job.
- **Hold SHIFT** as well for finer steps on the mixer's faders, pans and sends, an effect's knobs and sliders, and markers (one frame at a time, not snapped). Press SHIFT after your finger is down: SHIFT and a touch on a value is MIDI Learn.
- **A tap is still a tap.** If you don't turn the wheel, the touch does what it always did. Once you have turned it, lifting your finger presses nothing: a value you can type doesn't open the number pad, and a macro card held still doesn't open the macros panel if you turned the wheel first.
- **Dragging** a value and turning the wheel both move it. Dragging a list scrolls it and gives the wheel back.
- **Two fingers:** the wheel turns the one you put down last. Lift it and the wheel goes back to the first.
- **The push** does nothing while your finger is down, so a slip of the thumb can't press anything.
- **Undo:** on the Sample and Pads tabs a whole hold is one UNDO. On the Synth tab and the mixer, turns of one control less than a moment apart are one UNDO.

## Undo

**UNDO** takes back your last recording or edit on the page you are on, and the hint line says what it took back: on the Looper tab the last loop layer, on the Sequencer and Arrange tabs the last take (a recording, a step edit, a deleted clip), on the Sample and Pads tabs the last edit, on the Synth tab and the mixer the last thing you turned. **SHIFT + UNDO** brings it back. While a recording is going, UNDO throws the take away and the music plays on.

Hold **UNDO** for a moment for the page's **undo history**: every step, oldest at the top, with **now** beside where you are and the steps you undid faint. Tap a step (or turn the wheel to it and push) to go straight back, or forward, to just after it.

{{shot: undo-history | The Sequencer's undo history, opened by holding UNDO: four takes and edits done, two undone (faint). Tap a step to go back or forward to just after it.}}

## Touch

- **Tap** to press a button, pick a tab, a pad or a step. Anything you can tap answers over its whole button, and packed rows (pattern lanes, list rows) answer over the full row with no gap between them.
- **Drag** to move a knob, a fader or the XY pad, or to scroll a list.
- **Hold** a throw or the XY pad to play it for as long as you hold.

If taps land in the wrong place, calibrate the screen from Prefs (**Calibrate touch**).

> [!MPC]
> There is no Main mode with tracks and programs on screen, and the Play page has no pads on screen: you play the real ones, and the screen uses the room for the loops, the pattern and performance tools. Prefs is not a menu item: it opens with **SHIFT + MENU**. Every page shows what the Q-Links do in the column beside them, so there is no Q-Link Edit mode.
