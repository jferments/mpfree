<!--
status: changing
covers: rust/mpfree-app/src/xy_song.rs rust/mpfree-app/src/xy_setup.rs rust/mpfree-app/src/xy_widgets.rs rust/mpfree-app/src/xy_list.rs rust/mpfree-app/src/xy_play.rs rust/mpfree-app/src/xy_own.rs
checked: 2026-11-23 a9ea20d
-->

# Your own XY layouts

The [XY pad](playing-live.md#the-xy-pad) comes with ten ready-made layouts: Filter + echo and the nine XY FX. You can also make your own. A layout of yours puts any settings you like on the pad: up to eight move as your finger goes across, and up to eight more as it goes up. For example, a filter's cutoff and a distortion's drive across, and a send and an echo's mix up, so one finger opens up a bass line and throws it into the echo. Your layouts are kept in the song, so they come back when you open it.

A layout can hold a synth's settings (the synth on the pads, or a track's own instrument), a pad's own settings (its level, pan, tune, filter and envelope), the mixer's settings (a strip's volume, pan and sends), any effect's settings, the macros and outside gear's knobs. The arp's settings come soon.

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

## Setup

Setup shows the layout's name, each direction with its name and its settings, and how the pad behaves when you let go. Tap the name to type the layout's name and what across and up are called.

{{shot: play-xy-setup | Setup for Squelch: three settings across, two up (one on a removed track, struck out), Cutoff picked for the Q-Links.}}

Each setting has a row:

- **The range.** Drag either handle on the bar to set where the setting is at each end of the pad. Put the right-hand end lower than the left and it runs backwards. Tap a row to pick it: **Q1** and **Q2** then set the two ends, and **Q3** its curve.
- **The curve:** Even, Slow start (most of the change at the far end, good for a filter's cutoff), Fast start or S-shaped.
- **Set or Add.** Set moves the setting itself, between the two ends. Add pushes it up or down from wherever it is, so it works on top of a knob you've turned, an LFO or a step lock. Add always glides back when you lift. A synth's or a pad's own setting moves by Set for now: Add is greyed on it.
- **×** takes the setting off the layout.

A setting that went away (its track was removed, or its effect taken out) is struck out with the reason. It does nothing until you take it off with **Take off**.

At the foot:

- **When you let go:** **Go back** glides each setting back to its own value (where its knob, a step lock or an LFO puts it), over the **Glide back** time (**–** and **+**, or **Q4**). **Stay** leaves the settings where you lifted, as if you had turned their knobs, and **UNDO** puts them back.
- **A touch:** **Jumps** puts the pad under your finger. **Moves from here** moves it from where it is by as much as your finger moves, so nothing jumps when you land.
- **Follow the selected track** is for a synth's settings, and comes soon.

**Delete** takes the layout out of the song. **Done** takes you back to the pad.

## Hold and Let go

Under the full-screen pad (**SHIFT + NEXT SEQ**) are three buttons:

{{shot: play-xy-full-yours | Squelch on the full-screen pad, with Hold, Let go and Setup under it.}}

- **Hold** keeps the pad in charge when you lift your finger: the settings stay where they are until you turn Hold off, and then they let go as the layout says. It works on layouts of yours. STOP pressed while the music is stopped turns it off.
- **Let go: Go back** or **Let go: Stay** switches what the pad does when you lift. On a ready-made layout, a tap makes a copy of it yours with the other choice.
- **Setup** opens Setup for the layout playing (on a ready-made one, a copy becomes yours first).

UNDO on the full-screen pad and in Setup undoes what Stay left behind, one lift at a time. Hold UNDO to see the list.

A button, a key or a footswitch on your controller can switch Hold and step through your layouts: learn it on **XY Hold**, **XY layout next** or **XY layout back** in the [Learn section's Play card](midi-and-cv.md#plays-swing-filter-and-xy-pad-from-your-controller).
