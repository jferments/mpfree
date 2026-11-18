<!--
status: changing
covers: rust/mpfree-ui/src/widgets.rs rust/mpfree-ui/src/qlink.rs rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/gui.rs
checked: 2026-11-16 a994ab4
-->

# The screen

Every page in MPFree is laid out the same way, so once you know one, you know them all.

{{shot: play | The Play page. The tabs are at the top left, the Q-Link column at the right, and the hint line along the bottom.}}

## The top bar

**The tabs** on the left are MPFree's main pages. Tap one to go there:

| Tab | What it is for |
|---|---|
| **Play** | The home page: finger drumming, beats and live looping. |
| **Sequencer** | The armed track's clip, as steps (**Steps**) or as notes (**Notes**, the note editor). **STEP SEQ** opens it. |
| **Looper** | Up to eight loops side by side: record, overdub and mute each with a tap, and edit the layers of the one you pick. |
| **Sample** | Recording, chopping and shaping the sound on one pad. |
| **Synth** | MPFree's synths and their sounds, or an MPC plugin's own screen ([The plugin's own screen](../making-music/synths.md#the-plugins-own-screen)). |
| **Arrange** | Clips: recording them, keeping takes and launching them. |
| **Pads** | The kit's 16 pads beside the pad's settings: set each pad up, and **Browse for** a sound to put on it. |
| **Mix** | The mixer and every sound's effects (next build). See [Mixing and effects](../making-music/mixing-and-effects.md). |

**The status chips** on the right show what is going on: how much of the processor (**CPU**) and memory (**RAM**) is in use (these two keep their width as the numbers change, so the chips beside them stay put), whether Full Level is on, and anything that needs your attention. When memory runs so short that new sounds can't load, the hint line says so (see [Sampling](../making-music/sampling.md#the-pads-tab-sounds-onto-pads)). What the pads play is on the Play page's transport bar (below). Two of the chips do something when tapped:

- **The Full level or Half level chip** turns that level off, as pressing **FULL LEVEL** again would: the pads play as hard as you hit them again.
- **The Pads: steps chip** (such as **Pads: steps · Kick**) shows while **STEP SEQ** has made the pads the steps of a sound. With a synth on the pads it names the synth and the note or chord new steps play (**Pads: steps · Poly · C3 + G3**). Tap it to play the pads again, as pressing **STEP SEQ** again would. See [Program a pattern on the pads](first-beat.md#program-a-pattern-on-the-pads).

Tap any other chip and the hint line says what it shows.

The small **folder** at the far right opens the browser over the page, as **BROWSE** does, and tapping it again closes it. The mixer's I/O view, where the project's audio, MIDI, sync and CV are routed, is on the **Mix** tab (tap **I/O** there). While the tempo follows a clock from another device, a tempo chip such as **120.0 from MIDI In** shows it, with a green dot; tapping it opens the I/O view too.

## The transport bar

On the Play page and the Sequencer tab, a bar under the tabs shows time: whether MPFree is stopped, playing or recording, the position (bar, beat and step), the tempo in BPM (tap it in time to set the tempo, as with **TAP TEMPO**, or hold a finger on it to type the tempo), and what **REC** records. On the right of it is the view switch: **All** and **Loop** on Play (see [Playing live](../making-music/playing-live.md)), **Steps** and **Notes** on the Sequencer.

On Play, **PADS PLAY** before the switch says what the pads play: the selected track, its kit and the pad bank, such as **Drums · Boom Bap · A** (or the synth, such as **Keys · Poly · C3 to D#4**). Tap it for the list of kits. While controller mode is on, the pads play your outside gear, so it says what the **Controller** chip at the top says, such as **Controller → Juno-X · ch 3 · A** (see [Controller mode](../making-music/midi-and-cv.md#controller-mode-the-pads-and-q-links-play-your-gear)). On the Sequencer, dots that light up with each bar and beat stand there instead.

**The list of kits** opens over the page from **PADS PLAY** on Play or **Kit** on the Pads tab. Tap a kit to load it on the pads; the list closes and you are back where you were. The synth comes after the kits, and the last row, **New kit**, puts an empty kit on the selected track. **Back** or any tab closes the list and keeps the kit that is playing.

## The Q-Link column and the master bar

At the right edge, the **Q-Link column** shows four cards, one for each Q-Link knob beside the screen, in the same order (knob 1 at the top). Each card shows the knob's number and what it controls, centred over a ring with its value in it. Turn the knob, or drag up and down on its card.

Next to it, the **master bar** shows how loud MPFree's output is, with a meter that holds its peak. Drag the handle to set the overall level. It starts at -6 dB, the level the browser auditions sounds at, so a sound plays as loud on the pads as it did when you heard it in the browser, and goes up to 0 dB. "LIM" lights when the limiter is stopping the output from distorting.

## The hint line

The line along the bottom of the screen always says what the buttons do on the page you are on. If you're not sure what to press, look there first.

When something you asked for can't happen, or does nothing, the hint line says why in amber for a few seconds, then goes back to the hint. For example: "The chain is full", or why a sample edit was refused.

The same goes for a faint button or card: tap it and the hint line says why it is faint and what to do first, such as "Copy a pad first" or "The filter is off: pick one in the Filter row". A setting that is already as low or as high as it goes says so too, such as "Count-in is off already". A tab, row or card that is already lit has nothing new to show, so a tap on it changes nothing.

## Things that open over the page

Some things are not tabs. They open over the page you are on, and close the same way they opened:

- **Options:** press **MENU** for the current page's settings, such as the repeat rate or the tempo on the Play page.
- **Prefs:** press **SHIFT + MENU** for MPFree's settings. See [Prefs](../reference/prefs.md).
- **The browser:** press **BROWSE**, or tap the folder at the top right. See [Saving your work](../making-music/saving.md#the-browser).
- **The mixer's I/O view:** tap **I/O** on the **Mix** tab. See [the I/O view](../making-music/mixing-and-effects.md#the-io-view). (The mixer itself is the **Mix** tab from the next build; on the current test build it opens over the page with **TRACK MIX** or the sliders icon.)
- **The Arp panel:** hold **NOTE REPEAT** and press **MENU**.
- **The list of kits:** tap **PADS PLAY** on Play's transport bar, or **Kit** on the Pads tab.

{{shot: kits | The list of kits, opened from PADS PLAY. Tap a kit to load it on the pads; Back closes the list.}}

{{shot: options | Play's options, opened with MENU. Turn the wheel or tap to pick a row; – and + change it.}}

In a list of settings like this one, turn the data wheel or tap to pick a row, and press **–** or **+** (or tap the arrows) to change it. Or push the data wheel: the row's value is outlined, the wheel now changes it, and a second push sets it so the wheel moves between rows again. The panel on the right explains the row you picked. A list longer than its panel scrolls a row at a time as you pick, and a finger can drag it up or down; the thin bar at its right shows where you are.

The settings beside the Sample tab's pages and on the Pads tab, the Synth tab's tiles and Arrange's Record settings work the same way: the wheel picks, **–** and **+** change, and the push lets the wheel change it. Holding **SHIFT** gives the wheel its other job on that page (the next pad, the next page, scrolling).

## The wheel's outline

On the Play page, the Looper tab, the mixer and the FX chain editor, the data wheel takes you round the screen without touching it (next build):

- **Tap a control** and an outline appears on it. **Turn the wheel** and the outline moves to the next control, left to right and top to bottom through that part of the screen, then on into the next part. On Play and the Looper tab you can also just turn the wheel: the outline starts on the first control.
- **Push the wheel** to press the control the outline is on, as a tap would. On a throw, the push holds it on until you push again.
- **On a value** (a fader, a pan, a send, a Q-Link card, the master level, an effect's knob), the push grabs it: the outline gets thicker and the wheel turns the value. Push again to let go, and the wheel moves the outline again. **–** and **+** step an outlined value without the push.
- **In the chain editor**, push on the picked effect to pick it up: the wheel (or **–** and **+**) moves it up or down the chain, and a push puts it down.
- Some values can't be turned with the wheel yet (the XY pad, the Looper tab's trims); the line at the bottom says to drag them.
- On the mixer and the chain editor, the wheel keeps its old job (the selected strip's volume, the first knob) until you tap a control. Changing page, or tapping an empty spot, takes the outline away.
- While the list of effects is open, the wheel moves through the effects and its push opens presets and then adds, as always (see [Adding an effect](../making-music/mixing-and-effects.md#adding-an-effect)).

To change the kit, tap **PADS PLAY** on Play (or **Kit** on the Pads tab), or hold **SHIFT** and turn the wheel on the Play page. The **PAD BANK** buttons put the kit's other banks of 16 pads on the pads.

## Hold and turn

On every page, **rest a finger on anything with a value and turn the data wheel**: the wheel turns that value for as long as your finger stays down, and **–** and **+** step it. It gets a thick outline, and the line at the bottom of the screen says what it is and what it reads now. Lift your finger and the wheel goes back to its usual job on that page (picking rows, moving the outline, zooming). There is no wait: the wheel takes the control as soon as your finger is down (next build).

- **What it turns:** knobs, faders, pans and sends, sliders, Q-Link cards and macro cards, an effect's knobs, the Synth tab's knobs, choices and switches, the settings rows of the Sample and Pads tabs, Arrange and Prefs (the whole row, not just its arrows), and the Wave page's markers. A marker moves a screen pixel a turn at the zoom showing, snapped as its Q-Link moves it. On buttons, pads, tabs and list rows that only pick something, the wheel keeps its usual job.
- **Hold SHIFT** as well for finer steps on the mixer's faders, pans and sends, an effect's knobs and sliders, and markers (one frame at a time, not snapped). Press SHIFT after your finger is down: SHIFT and a touch on a value is MIDI Learn.
- **A tap is still a tap.** If you don't turn the wheel, the touch does what it always did. Once you have turned it, lifting your finger presses nothing: a value you can type (dotted line) doesn't open the number pad, and a macro card held still doesn't open the macros panel if you turned the wheel first.
- **Dragging** a value and turning the wheel both move it. Dragging a list scrolls it and gives the wheel back.
- **Two fingers:** the wheel turns the one you put down last. Lift it and the wheel goes back to the first.
- **The push** does nothing while your finger is down, so a slip of the thumb can't press anything.
- **Undo:** on the Sample and Pads tabs a whole hold is one UNDO. On the Synth tab and the mixer, turns of one control less than a moment apart are one UNDO, as always.

## Touch

- **Tap** to press a button, pick a tab, a pad or a step.
- **Drag** to move a knob, a fader or the XY pad.
- **Hold** on a throw or the XY pad plays it for as long as you hold.

> [!MPC]
> There is no Main mode with tracks and programs on screen, and the Play page has no pads on screen: you play the real ones, and the screen uses the room for the loop, the pattern and performance tools. Prefs is not a menu item: it opens with **SHIFT + MENU**.
