<!--
status: changing
covers: rust/mpfree-fx rust/mpfree-mixer-ui rust/mpfree-fx-ui rust/mpfree-chain-ui rust/mpfree-app/src/plugins.rs rust/mpfree-plughost/src/rack.rs rust/mpfree-app/src/chain_page.rs rust/mpfree-app/src/mixing.rs rust/mpfree-fx/src/perform.rs
checked: 2026-11-13 9560b32
-->

# Mixing and effects

Every sound in MPFree, from the pads, the synth, the loop and the line input, meets in one **mixer**. There you balance levels, place sounds left and right, send them to shared effects such as a delay, and group them. Effects can sit on any one sound, on a group, or on the whole mix.

## Opening the mixer

Press **TRACK MIX**, or tap the small mixer icon at the top right of the screen. The mixer opens over the page you are on, so the music and the page carry on underneath. Press TRACK MIX again to close it. **SHIFT + TRACK MIX** opens it straight on the pads.

{{shot: mixer | The mixer, shown here with example tracks from the preview: inserts at the top of each strip, sends A to D, pan, the fader and meter, mute and solo, and where the strip goes. The master is on the right, ending in Master safety.}}

Along the top are the mixer's five views:

| View | Shows |
|---|---|
| **Tracks** | One strip per instrument: **Drums** (the kit on the pads), **Synth**, **Loop** and **Audio** (every audio track on the Song view, summed), then a strip for each track that has its own instrument (**Track 1**, **Track 2** ...), shown only while it has one (see [A synth for each track](synths.md#a-synth-for-each-track)). |
| **Pads** | A strip for each of the kit's 16 pads, so you can set the kick's level or put an effect on just the snare. |
| **Inputs** | The line input. |
| **Submixes** | Eight submixes for grouping, then the four send returns A to D. |
| **I/O** | Where everything goes, as a list: see [The I/O view](#the-io-view). |

{{shot: mixer-instruments | The Tracks view with tracks 1 and 2 playing their own instruments (Poly and FM): their strips come after Drums, Synth, Loop and Audio.}}

The **< >** buttons page through the strips when there are more than fit. The chip beside them shows how much of the MPC's processor the mixer and its effects are using.

## A channel strip

From top to bottom, each strip has:

- **Inserts**: up to four effects that the sound goes through, in order.
- **Sends A to D**: how much of the sound goes to each send return. A return holds a shared effect, so several strips can use one reverb or delay.
- **Pan**: where the sound sits between left and right.
- **The fader** (from silent up to +6 dB) and the **meter** beside it.
- **M** (mute) and **S** (solo). Solo keeps the strip and everything it feeds or is fed by, so soloing the snare's pad still lets you hear it through the Drums track.
- **The output**: where the strip goes. A pad goes to its track; a track to the master or a submix; a submix to the master or a higher-numbered submix.

Drag a fader, pan or send with your finger, and tap M, S or the output to change them.

## The I/O view

**I/O** shows the project's routing in one place instead of strip by strip. The master stays on the right; down the left is a list of sections, and the section you pick fills the rest.

To open it, tap the small **I/O icon** (an arrow in over an arrow out) just left of the mixer icon in the top bar, or tap **I/O** among the mixer's views. The I/O icon lights while the view shows; tap it again, press **TRACK MIX** or pick a tab to close the mixer. While the tempo follows a clock from another device, the top bar shows a tempo chip such as **120.0 from MIDI In** with a green dot; tapping it opens this view too, at **Sync** once that section is there.

- **Audio** has a row for every strip that has an output: the inputs, the tracks, the pad channels and the submixes, each with where it goes. An input's row also has its **Source** and **Monitor**.
- Tap a row to select that strip (the Q-Links follow it). Tap its output to open the same picker as on the strip. Tap an input's Source or Monitor to step through the choices.
- Turn the **data wheel** to move the selection up and down the list, and press **-** or **+** to step the selected row's output.
- An output of **None (silent)** shows in amber, so a strip that goes nowhere is easy to spot.

Under Audio come three more sections, for the gear around the MPC (see [Playing other gear](midi-and-cv.md)):

- **MIDI** lists the ports plugged in, each with what its **Input** does and its **Thru**, then **Tracks out**: where each track plays (**Plays on**, the port and the channel together, or **Inside MPFree**) and the **Program** it sends.
- **Sync** sets **Tempo from** (MPFree itself or a port's clock), what following does with start, stop and the position, the clock arriving now, and which ports send **Clock out**.
- **CV** has a row for each of the eight CV/gate outputs: its **Job**, the **Track** it follows and its setting, with the volts it gives now.

A setting for a device that isn't plugged in, or for a part of MPFree that doesn't work yet, says so on its own row, and is kept. The MIDI, Sync and CV settings are saved with the project, and so is the rest of the mixer: levels, pans, sends, mutes and every effect with its settings (see [Saving your work](saving.md#projects)). The line input's source, monitoring and arming are not part of a project, so opening one never turns the input on.

## Using the Q-Links in the mixer

Tap a strip's name to select it. The four Q-Links then turn its **Volume**, **Pan**, **Send A** and **Send B**, and the data wheel turns its volume too.

## The master and Master safety

The **Master** strip on the right is everything you hear. Its fader is the same level as the master bar on the Play page and Q-Link 1 there.

The last thing on the master is **Master safety**, a limiter that keeps the output from going past -0.3 dB, so a loud moment never distorts harshly or hurts your ears. It shows how many times it has caught a peak. If it is catching all the time, turn something down.

> [!TIP]
> Distortion you want is fine: put Drive, Crush or Lo-fi on a strip and push it. Master safety only stops the final output from clipping by accident.

## The effects

These effects can go on any strip. The list of effects groups them: Dynamics, EQ and filter, Drive and colour, Modulation, Delay and reverb, Pitch and vocal, and Performance.

| Effect | What it does |
|---|---|
| **EQ** | Shapes the tone: cut mud, add air, thin out a sound. |
| **Multi filter** | Low-pass, high-pass, band-pass and more, with resonance. |
| **DJ filter** | One knob: turn left for a low-pass, right for a high-pass. |
| **Drive** | Warm saturation up to hard distortion. |
| **Comb** | Short tuned delays for metallic, resonant tones. |
| **Crush** | Bit and sample-rate reduction. |
| **Delay** | Echoes in time with the tempo. |
| **Chorus**, **Flanger**, **Phaser** | Movement and width. |
| **Lo-fi** | Old sampler and worn tape character. |
| **Compressor** | Evens out levels and adds punch. It can be keyed from another strip, for side-chain pumping. |
| **Ducker** | The pump: turns the sound down on each kick (keyed from another strip) or on every 1/16, 1/8, 1/4, 1/2 or bar of the tempo, and brings it back on a curve you shape. |
| **Transient shaper** | More or less punch at the start of each hit and more or less ring after it, at any level, with no threshold to set. |
| **Pitch shifter** | Moves the sound up to two octaves up or down without changing its speed. Spread detunes left and right for a wide double; feedback makes rising or falling stairs. |
| **Cloud reverb** | A large, lush reverb. |

Adding, removing or reordering an effect while the music plays doesn't click: that strip dips out for a moment and comes back.

## Plugins

MPFree can also use **CLAP plugin effects** built for the MPC One's ARM chip. Plugins made for a computer (Windows, macOS or ordinary Linux PCs) won't run on it.

1. Put the `.clap` files on the MPFREE-DATA partition of your MPFree stick, in the folder `files/plugins/`.
2. Start MPFree. The plugins' effects join the list you add effects from, under **Plugins** after the built-in ones, each with its maker's name.
3. Add one to a strip as you would any effect. Its knobs show in the chain editor, in the plugin's own groups, and the Q-Links, macros and MIDI Learn work on them.

Each plugin runs on its own, apart from the rest of MPFree, so a plugin that crashes or freezes can't stop the music:

- If a plugin is too slow for a moment, its strip plays dry (the sound goes straight through) for that moment.
- If a plugin crashes, MPFree starts it again with the settings it had a few seconds before.
- If it crashes four times in a minute, MPFree leaves it off and the strip plays dry. Remove it and add it again to try once more.

Every one of these is written in MPFree's report, so nothing goes wrong without you hearing about it.

> [!NOTE]
> A plugin's settings are saved with your project. If you open the project on a stick without that plugin, the effect is left out and the Events log names the plugin's file. Plugin synths play as a track's own instrument (see [Synths](synths.md#a-synth-for-each-track)). **Not yet:** MPFree doesn't come with any plugins yet. A pack of free plugins built for the MPC (Airwindows, Surge XT and Dexed) is coming.

## The chain editor

Tap an effect in a strip's insert row to open the **chain editor** on it, in the mixer's place. Tap the strip's **+ Insert** to add an effect, with the list of effects already open; a new effect goes at the end of the chain. In the chain editor, **+ Add effect** opens the list again and **Cancel** closes it; you can also reorder (**Up**, **Down**), bypass (the power button on each row) and **Remove** effects, and turn the selected effect's knobs. If the chain is full or the mixer is busy, the line at the bottom of the screen says so for a few seconds.

The Q-Links turn the selected effect's first four knobs, and the data wheel works as Q-Link 1. **‹ Mixer** goes back to the mixer as you left it.

The Sample tab's **FX** opens the chain editor too, on the pad you are editing (its own channel, then the Drums track), and Back goes back to the Sample tab (see [Sampling](sampling.md#fx-effects-on-one-pad-or-on-the-whole-kit)).

To give a knob a control of its own, touch it and tap **Learn to a Q-Link**, then turn a Q-Link, or move a knob or fader on your keyboard (see [MIDI Learn](midi-and-cv.md#midi-learn)).

{{shot: chain | The chain editor on the drum bus, with Drive selected.}}

## Macros

**Macros** are knobs that move several settings at once (Wash, Grit and so on). There are eight. The **Q-LINK** button steps the Q-Links through **Macros 1-4**, **Macros 5-8** and back to the page's own knobs, on the Play page and in the chain editor.

Hold a finger still on a macro's card for half a second to open the **macros panel**, where you can see all of them and set up what each one moves. Back returns to the page you were on.

{{shot: macros | The macros panel on Macros 1-4.}}

## The Play page's effects

Some effects are already in place when MPFree starts, and the Play page's Q-Links and touch controls play them:

- **A DJ filter** on the Drums and Loop tracks: the **Filter** Q-Link on the Play page.
- **A dotted-eighth delay** on send return A: the **Delay send** Q-Link.
- **The throws and the XY pad** on the master: see [Playing live](playing-live.md#throws).

Each of these costs almost nothing until you use it.

## Coming next

- **Plugins**: the plugin pack.
- **Automation**: knob moves recorded into clips.
- **Perform FX on pads**: effects played from the pads.

> [!MPC]
> The mixer covers the MPC's routing: pad channels, tracks, submixes, four send returns and the master, with sends before or after the fader and side-chain keys. The differences: the mixer opens over any page instead of being its own mode, solo keeps everything the strip feeds, the master always ends in a safety limiter, and effects with a delay of their own are lined up automatically so nothing drifts out of time.
