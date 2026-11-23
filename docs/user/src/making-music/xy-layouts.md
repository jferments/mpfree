<!--
status: changing
covers: rust/mpfree-app/src/xy_song.rs rust/mpfree-app/src/xy_setup.rs rust/mpfree-app/src/xy_widgets.rs rust/mpfree-app/src/xy_list.rs rust/mpfree-app/src/xy_play.rs rust/mpfree-app/src/xy_own.rs
checked: 2026-11-23 a9ea20d
-->

# Your own XY layouts

The [XY pad](playing-live.md#the-xy-pad) comes with ten ready-made layouts: Filter + echo and the nine XY FX. You can also make your own. A layout of yours puts any settings you like on the pad: up to eight move as your finger goes across, and up to eight more as it goes up. For example, a filter's cutoff and a distortion's drive across, and a send and an echo's mix up, so one finger opens up a bass line and throws it into the echo. Your layouts are kept in the song, so they come back when you open it.

A layout can hold a synth's settings (the synth on the pads, or a track's own instrument), a pad's own settings (its level, pan, tune, filter and envelope), the mixer's settings (a strip's volume, pan and sends), any effect's settings, the macros, outside gear's knobs and a track's arp (its rate, gate, swing, octaves and Modifier).

## Making a layout

There are two ways to start:

- **From nothing.** Tap the XY pad's chip, then **+ New layout** under YOURS · IN THIS SONG. An empty layout called "Layout 1" plays, and its Setup opens.
- **From a ready-made layout.** Press **SHIFT + NEXT SEQ** for the full-screen pad, pick the ready-made layout you want from its chip, and tap **Setup**. A copy becomes one of yours and opens in Setup, with the same settings and the same feel. The ready-made one stays as it was.

{{shot: play-xy-yours | The XY pad's list with two layouts of yours under the ready-made ones: Squelch is playing.}}

To play a layout of yours later, tap the chip and tap its tile. The chip names it, and the pad's axes carry the names you gave across and up.

## Adding settings

In Setup, tap **+ across** or **+ up**. A banner comes up at the foot of the screen and stays on every page: "Bite on Squelch waits: hold SHIFT and touch settings on any page". Now go anywhere (the mixer, an effect in the chain editor, the Synth tab, a pad's knobs on the Sample tab's Play page, the macros) and **hold SHIFT and touch a setting**. It joins that direction at once, over its whole range, and the banner's faint line says what just happened. Press **SHIFT + NEXT SEQ** to try the pad, and come back for more.

{{shot: xy-banner | The waiting banner: settings touched with SHIFT now join Bite, across, on Squelch.}}

- A setting already on the other direction moves over, and the banner says so.
- A switch can't go on the pad, and a direction already holding eight settings is full. The banner says why.
- **Done** keeps what you added. **Cancel** puts the layout back exactly as it was.

While the banner is up, SHIFT + a touch picks for the pad instead of for MIDI Learn. Once it's gone, SHIFT + a touch is MIDI Learn again.

### From the chain editor

In the chain editor, touch an effect's control and tap **XY →** or **XY ↑** on the Modulate row under the page. The control goes on that direction of the layout the pad plays, over its whole range, and the hint line says so: "Tone (Drums strip · Drive, effect 2) moves up on Squelch, 5.0 to 20 kHz · XY ↑ again: take it off · its range and curve: Setup on the XY page".

- On a ready-made layout, a layout of yours is made from it first, and the hint line says so. The ready-made one stays as it was.
- The chip is lit while the control is on that direction. Tap the lit chip to take it off; tap the other one to move it there.
- A switch can't go on the pad: its chips are greyed, and a tap says why.

Every control on the layout the pad plays wears a mark on the chain editor's pages and, for a synth's settings, on the Synth tab: **X→** or **Y↑** at its knob's top left (under a macro's badge), a thin arc outside its ring from a tick at the layout's from to an arrow at its to (a backwards one's arrow runs the other way), and a dot where the setting really is now. For an Add setting the arc starts from the setting's own value, and the dot is its value plus the pad's push.

{{shot: synth-xy | The Synth tab's Filter page with a layout of yours playing: Cutoff on across, Resonance on up running backwards.}}

### The arp on the pad

Open the Arp panel (**NOTE REPEAT + MENU**) while the banner waits, and **hold SHIFT and touch** its **Rate**, **Gate**, **Swing** or **Octaves** slider, or the **Modifier** box. (With **NOTE REPEAT** held, the Q-Link cards are the arp's four, and SHIFT + a touch on one works too.) It joins the layout for the armed track's arp.

- **While your finger is down** the arp plays what the pad says, from its next step, and the panel shows it with a ring, as it does for the Modifier. The settings you set stay as they are: lift, and Go back plays them again.
- **The Modifier** on the pad pushes the arp by the Modifier's amounts (two rates faster and an octave up, unless you've changed them), even with no controller picked. While the pad holds it, the Modifier's own controller waits; lift, and the controller has it back.
- **Turning a held setting** (its Q-Link or the panel) takes it from the pad until you lift.
- **Stay** keeps the pad's rate, gate, swing or octaves as the arp's own, and **UNDO** puts them back. The Modifier's push stays where you left it until its controller moves (with no controller picked, it ends when you lift).
- Swing held by the pad is the arp's own swing, even while the sequencer swings.

## Setup

Setup shows the layout's name, each direction with its name and its settings, and how the pad behaves when you let go. Tap the name to type the layout's name and what across and up are called.

{{shot: play-xy-setup | Setup for Squelch: three settings across, two up (one on a removed track, struck out), Cutoff picked for the Q-Links.}}

Each setting has a row:

- **The range.** Drag either handle on the bar to set where the setting is at each end of the pad. Put the right-hand end lower than the left and it runs backwards. Tap a row to pick it: **Q1** and **Q2** then set the two ends, and **Q3** its curve.
- **The curve:** Even, Slow start (most of the change at the far end, good for a filter's cutoff), Fast start or S-shaped.
- **Set or Add.** Set moves the setting itself, between the two ends. Add pushes it up or down from wherever it is, so it works on top of a knob you've turned, an LFO or a step lock. Add always glides back when you lift. A synth's, a pad's or an arp's own setting moves by Set for now: Add is greyed on it.
- **×** takes the setting off the layout.

A setting that went away (its track was removed, or its effect taken out) is struck out with the reason. It does nothing until you take it off with **Take off**.

At the foot:

- **When you let go:** **Go back** glides each setting back to its own value (where its knob, a step lock or an LFO puts it), over the **Glide back** time (**–** and **+**, or **Q4**). **Stay** leaves the settings where you lifted, as if you had turned their knobs, and **UNDO** puts them back.
- **A touch:** **Jumps** puts the pad under your finger. **Moves from here** moves it from where it is by as much as your finger moves, so nothing jumps when you land.
- **Follow the selected track:** a synth's settings on the layout then mean the synth of whichever track you've selected, so one layout plays the bass's filter on the bass track and the lead's on the lead track. The arp's settings mean that track's arp too. What a touch plays is fixed when your finger lands: selecting another track while you play changes the next touch, not this one. The mixer's and the effects' settings stay where they are.

**Save to browser** keeps a copy of the layout in the browser for your other songs (see below). **Delete** takes the layout out of the song. **Done** takes you back to the pad.

## Using a layout in another song

A layout lives in the song it was made in. To use it in other songs too, tap **Save to browser** at the top of its Setup. The browser keeps it under **Instruments › XY pad**, by its name. Saving it again under the same name replaces the copy there, and the old copy goes to the Trash, so you can get it back.

In another song, tap the XY pad's chip, then **From the browser** next to + New layout. The browser opens on your saved layouts. Turn the wheel to one and push it (or tap **Load**): it becomes one of yours in this song, playing, and you're back on Play. **Cancel** or **BROWSE** comes back with nothing changed. If the song already has a layout of that name, the new one is numbered ("Squelch 2"). You can also load a saved layout from **Instruments › XY pad** in the full browser.

{{shot: browser-xy-layouts | From the browser: three layouts saved from other songs, Wobble picked.}}

A layout brings what it can into the new song:

- The mixer's and the effects' settings come where this song has the same setting in the same place: the same strip's volume, or the same effect in the same slot. A setting this song doesn't have there, or has a different setting in its place, is left out, and the hint line names what was left out. It is never put on whatever happens to be there.
- A copy of a ready-made XY FX layout plays this song's XY FX. If the pad hasn't played that effect in this song yet, the hint line asks you to play it once first.
- On a layout that follows the selected track, a synth's settings come as they are: they mean the selected track's synth, so these layouts travel best.

## Hold and Let go

Under the full-screen pad (**SHIFT + NEXT SEQ**) are three buttons:

{{shot: play-xy-full-yours | Squelch on the full-screen pad, with Hold, Let go and Setup under it.}}

- **Hold** keeps the pad in charge when you lift your finger: the settings and the dot stay where they are until you turn Hold off, and then they let go as the layout says. It works on layouts of yours. STOP pressed while the music is stopped turns it off.
- **Let go: Go back** or **Let go: Stay** switches what the pad does when you lift. On a ready-made layout, a tap makes a copy of it yours with the other choice.
- **Setup** opens Setup for the layout playing (on a ready-made one, a copy becomes yours first).

UNDO on the full-screen pad and in Setup undoes what Stay left behind, one lift at a time. Hold UNDO to see the list.

A button, a key or a footswitch on your controller can switch Hold and step through your layouts: learn it on **XY Hold**, **XY layout next** or **XY layout back** in the [Learn section's Play card](midi-and-cv.md#plays-swing-filter-and-xy-pad-from-your-controller).
