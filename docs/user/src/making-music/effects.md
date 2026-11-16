<!--
status: changing
covers: rust/mpfree-fx rust/mpfree-fx-ui rust/mpfree-chain-ui rust/mpfree-app/src/chain_page.rs rust/mpfree-app/src/plugins.rs rust/mpfree-plughost/src/rack.rs rust/mpfree-mpchost rust/mpfree-fx/src/perform.rs rust/mpfree-fx/src/xyfx.rs rust/mpfree-fx/src/picker.rs rust/mpfree-chain-ui/src/picker.rs
checked: 2026-11-15 365d0f7
-->

# Effects

Effects change a sound: an EQ shapes its tone, a compressor evens it out, a delay adds echoes, a reverb puts it in a room. In MPFree an effect can sit on one pad, on a track, on a submix, on a send return that several strips share, or on the whole mix. This chapter covers adding effects, what each one does, macros, and plugins. Where effects sit in the mix is in [Mixing](mixing-and-effects.md).

## Adding an effect: the chain editor

Each strip in the mixer has up to eight **inserts**: effects the sound goes through, in order. Together they are the strip's **chain**.

{{shot: chain | The chain editor on the drum bus, with Drive selected.}}

1. Open the mixer (the **Mix** tab, or **TRACK MIX**) and find the strip.
2. Tap the strip's **+ FX** button. It fills the strip's empty insert rows, so on a strip with no effects it is a big button under the strip's name. The **chain editor** opens in the mixer's place, with the list of effects already open.
3. Pick an effect (see [Choosing an effect](#choosing-an-effect)). It goes at the end of the chain, selected, with its page showing.

Tap an effect already in a strip's insert row to open the chain editor on it. The strip's effects are down the left, the selected effect's page fills the middle, the Q-Links are at the right, and the **Modulate** row runs under the page.

- The line over the effects says how many the chain holds out of how many it can ("3 of 8"), and how much of the MPC's power the chain takes. Each effect's row has its number, its name, a line on how it is set (or "bypassed"), a dot for each macro that moves it, a **~** when an LFO or follower does, and its **ON** switch, which bypasses it.
- **+ Add effect** opens the list of effects again, and **Cancel** closes it.
- **Up** and **Down** move the selected effect in the chain, and **Remove** takes it out.
- To move an effect further, **drag it** up or down the chain: once your finger moves it lifts and follows you, a line shows where it will land, and it moves there when you lift. Lift it where it started to leave it where it was. With the wheel, push on the picked effect to pick it up, turn to move it, and push again to put it down.
- The effect's page is in **sections**, each named and coloured for what it does: the effect's heart (its space, its echoes, its pitch) in cyan, tone and colour in coral, dynamics in lime, movement in pink, and the output in grey. Drag a knob, fader or slider to turn it; tap a choice's name, or step it with its arrows; tap a switch to flip it.
- A control that does nothing as the effect is set shows dim, and its section says why: a delay's **Time** with **Sync** on, for one.
- Most effects show all their controls on one page. The few with more have a tab for each page beside the effect's name; if there are more tabs than fit, **‹** and **›** step to the page before or after. The Channel strip's first page shows its stages (**Gate**, **Comp**, **EQ**) as sections, each with its switch: tap a stage's switch to turn it on or off, or **Open ›** for its page.
- The four controls with a **Q1** to **Q4** badge are the effect's **Q-Links**, the four it is played by. They stay the same on every page, so you keep the same controls under your hands while you look through the rest; when a setting takes a control out of play (a delay's **Time** when **Sync** is on), its Q-Link turns the control in its place. The data wheel works as Q-Link 1 until you tap a control: then it moves the outline, and a push on a knob lets the wheel turn it (see [The wheel's outline](../getting-started/the-screen.md#the-wheels-outline)).
- **‹** at the top left goes back to where you opened the editor: the Mix tab as you left it, or the Pads tab.
- Beside **‹**, the **strip button** names the strip whose chain this is (with its colour, and "Track 7" or "Pad 3" under the name). Tap it for the **strip list**: the strips by group (**Tracks**, **Pads**, **Inputs**, **Returns**, **Submixes**, **Master**), with **< >** to page through a long group. Tap a strip to open its chain instead, on its first effect, or with the list of effects open if it has none. **Back** closes the list and keeps the strip you were on.

The drawing of what the effect does sits in the section it belongs to, and changes as you turn: an EQ's curve, a delay's repeats, the Ducker's pump over two beats (or after a kick), the Transient shaper on a drum hit (dashed going in, solid coming out), where the Pitch shifter puts the sound, with the steps its feedback climbs, Pitch correct's steps onto the key's notes (with a dot on the note you are singing), how far each Harmonizer voice moves on each note of the key, and the Tuner's needle.

Adding, removing or reordering an effect while the music plays doesn't click: that strip dips out for a moment and comes back. If the chain is full or the mixer is busy, the hint line says so for a few seconds.

### The Modulate row

Touch a control on the page and the **Modulate** row under the page names it. Then:

- Tap a **macro number** (1 to 8) to have that macro move the control, and tap it again to take it off. The control shows the macro's badge (**M2**) at its corner.
- Tap **Learn**, then turn a Q-Link, or move a knob or fader on your keyboard, to give the control a control of its own (see [MIDI Learn](midi-and-cv.md#midi-learn)).
- Tap **+ LFO** or **+ Follower** to have an LFO or an envelope follower move it. A ring round the knob shows how far it moves. The row opens the control's **mod rows** over the page: drag a row's bar to set how far and which way it moves the control, and tap **x** to take it off. The line under the control's name says what moves it ("LFO 1 · +30 %"); tap it to open the rows again. A touch anywhere else closes them. A switch takes macros only.

### Choosing an effect

**+ FX** on a strip, or **+ Add effect** in the chain editor, opens the list of effects over the chain editor. The line at its top says where the effect will go: which strip, and which slot of its chain.

- Down the left are **Favourites**, **Recent** (the last 12 effects you added), then the groups (Dynamics, EQ and filter, Drive and colour, Modulation, Delay and reverb, Pitch and vocal, Performance, Plugins, and Utility), each with how many effects it has. Tap one to see its effects on the right. The list opens where you left it.
- **Tap an effect** to add it with its usual settings.
- **Hold an effect** (half a second: its presets open while your finger is still down), or tap its **presets** line, to see its presets: tap one to add the effect already set that way, or **Defaults** for its usual settings. **‹ Effects** goes back.
- **Tap the star** on an effect to add it to Favourites, and again to take it off. Favourites and Recent are kept when you turn the MPC off, for every project.
- **With the data wheel:** turn it to move through the effects, **SHIFT + turn** to move through the groups, and **push** it to open the effect's presets; push again to add the preset that is lit. The hint line reminds you.

**Cancel** closes the list without adding anything.

The Pads tab's **Effects** row opens the chain editor on the pad picked, in the Mix tab, and **‹** comes back to the Pads tab (see [Sampling](sampling.md#the-pads-tab-sounds-onto-pads)). The Sample tab's **FX** page opens the same chain editor on the pad you are editing, with a switch between the pad's own channel and the Drums track (see [Shaping how a pad plays](shaping-a-pad.md#fx-effects-on-one-pad-or-on-the-whole-kit)).

## The effects

The list of effects groups them: Dynamics, EQ and filter, Drive and colour, Modulation, Delay and reverb, Pitch and vocal, Performance, and Utility (the Tuner, Stereo width and Sample delay). **Reverb**, **Delay**, **Drive**, **Filter** and **Tremolo** each come in several modes (see [Effects with modes](#effects-with-modes)).

| Effect | What it does |
|---|---|
| **Compressor** | Evens out levels and adds punch or glue. It can be keyed from another strip, for side-chain pumping. **SC filter** keeps low end (a kick) from pushing it, so a whole mix doesn't pump; **Detect** picks **Peak**, which catches every hit, or **RMS**, which follows the loudness and is smoother on a bus or a vocal. |
| **Ducker** | The pump: turns the sound down on each kick (keyed from another strip) or on every 1/16, 1/8, 1/4, 1/2 or bar of the tempo, and brings it back on a curve you shape. |
| **Transient shaper** | More or less punch at the start of each hit and more or less ring after it, at any level, with no threshold to set. |
| **Limiter** | A brick wall: nothing gets past the ceiling. **True peak** also catches peaks between samples; **Auto release** keeps a loud mix from pumping. |
| **Gate** | Silences (or turns down) what falls under the threshold, so drums stop ringing. It can open from another strip's sound, for rhythmic gating. |
| **De-esser** | Turns down harsh "s" and "t" sounds. **Listen** plays only what it hears, to find the right frequency. |
| **Multiband comp** | Three compressors, on the lows, mids and highs. |
| **Channel strip** | A gate, a compressor and an EQ in one, as on a mixing desk. Its first page, **Strip**, switches each one on or off and holds **Order** (the EQ before or after the gate and compressor) and **Output**; then a page each for the gate, the compressor and the EQ. |
| **EQ** | Shapes the tone with up to eight bands (each a bell, a shelf or a notch) and low and high cuts: cut mud, add air, take out a ring, thin out a sound. Its page is a graph with a dot for each band (see [The EQ's page](#the-eqs-page)); each band can be on (**ON**), switched off with its settings kept (**OFF**), or removed. Bands 1 to 4 start on and bands 5 to 8 start removed. |
| **Filter** | **Multi**: low-pass, high-pass, band-pass and more, with resonance. **DJ**: one knob, turn left for a low-pass, right for a high-pass. |
| **Comb** | Short tuned delays for metallic, resonant tones. |
| **Auto wah** | A wah that follows your playing: louder notes open it further. It can also sweep in time (**Source** LFO) or stay where you put it (**Source** Pedal: turn **Center** with a Q-Link, like rocking a pedal). |
| **Drive** | Warm saturation up to hard distortion: **Soft**, **Hard**, **Tube**, **Foldback**, **Fuzz** and **Rectify**; and the colour of a mixing desk or a tape machine: **Console**, **Tape**, **Iron** and **Soften**. Auto-gain keeps the level steady as you turn it up, so you hear more grit, not more volume. |
| **Amp** | A guitar or bass amp (Clean, Crunch, Lead, Bass) with its tone controls and a speaker cabinet. |
| **Exciter** | Adds sparkle and air to the top end. |
| **Crush** | Bit and sample-rate reduction. |
| **Lo-fi** | Old sampler and worn tape character: wear, wow and noise. |
| **Chorus**, **Flanger**, **Phaser** | Movement and width. |
| **Tremolo** | Volume (**Volume**) or position (**Auto-pan**) moved in time with the music. **Harmonic** is the swirl of 1960s amps. |
| **Ring mod** | Bells, clangs and robot voices. |
| **Rotary** | A spinning organ speaker: switch **Speed** between Slow and Fast and it ramps up or down like the real one. |
| **Freq shifter** | Moves every frequency up or down by the same number of hertz, so a sound stops being in tune with itself: a few hertz is a slow, endless swirl, more makes drums metallic and voices robotic. **Feedback** sends it round again, climbing or falling. |
| **Delay** | Echoes in time with the tempo. **Digital** is clean; **Tape** echoes from a tape loop with up to three heads, wow and flutter, that darken and thicken as they repeat; **Multi-tap** is four echoes on the beat, each with its own level and pan. |
| **Reverb** | **Room**, **Plate** and **Spring** are the smaller, classic reverbs (the room's **Gate** cuts its tail short, for the 1980s gated snare); **Hall**, **Cloud** and **Echoes** are large and lush. **Freeze** holds the tail for as long as it is on. |
| **Granulator** | Cuts the sound into tiny grains and plays them back as a cloud: higher or lower, scattered, backwards, spread wide. **Freeze** holds what's in it, so a moment hangs in the air while you play over it. |
| **Pitch shifter** | Moves the sound up to two octaves up or down without changing its speed. Spread detunes left and right for a wide double; feedback makes rising or falling stairs. |
| **Pitch correct** | Tunes a voice to the notes of a key. Pick the **Key** and **Scale**; **Speed** sets how quickly it pulls a note in (0 snaps straight to it, for the hard, stepped sound; 50 ms or more keeps slides and vibrato natural) and **Amount** how far. The **Note** and **Correction** readouts show the note it hears and how far it is moving it. |
| **Harmonizer** | Sings along with you: up to two extra voices a 3rd, 4th, 5th, 6th or octave above or below each note you sing, that stay in the key (a 3rd is major or minor as the key has it), each with its own level and pan. **Glide** smooths a voice into its next note, and **Dry** is your own voice. |
| **Talk box** | Makes a sound say vowels, from "oo" to "ee". The vowel moves with how loud you play or in time with the music; **Formant** makes the mouth smaller or bigger. Best on rich sounds: synth chords, organs, guitars. |
| **Perform FX** | The throws, stutters, beat repeats, tape stops and the XY pad, as an effect you can put on any strip. On the master it is what the Play page's throws play (see [Playing live](playing-live.md#throws)); on a pad, its 16 slots play stutters and repeats on that pad alone. |
| **XY FX** | One effect at a time played from two controls at once, X and Y: see [XY FX](#xy-fx). |
| **Tuner** | Tunes a guitar, a bass, a voice or a synth: put it on the strip the instrument comes in on (an input, usually) and play one note at a time. It shows the note, how many cents sharp or flat (the needle is green when you're within 3 cents) and the frequency. **Reference** sets the pitch of A, 440 Hz unless your band tunes to something else; **Mute** silences the strip while you tune. It doesn't change the sound and adds no delay. |
| **Stereo width** | Makes the sound narrower or wider, with the lows, mids and highs set apart: put the bass in the middle and open up the top. **Delay** widens a mono sound and still sounds right when the mix is played in mono. |
| **Sample delay** | Delays the left or the right side by a few samples, and flips either side's polarity: for lining up two microphones, or a quick stereo spread. |

### The EQ's page

{{shot: chain-mixer-drums-eq | The EQ on the drum bus: its curve with a dot for each band and the cuts, band 2 picked in the panel underneath.}}

The EQ's page in the chain editor is a graph of what the EQ does, from 20 Hz on the left to 20 kHz on the right and from -18 to +18 dB, with a dot for each band (numbered 1 to 8) and for the low and high cuts (**LC**, **HC**). The white line is what you hear; each band's shape is shaded in its colour.

- **Drag a dot** to move its band: sideways for its frequency, up and down for its gain. While you hold it, a bubble shows its values.
- **Turn the data wheel** on a dot, or put a **second finger** on the screen while you hold one, to make the band narrower or wider (its Q). On a cut, the wheel switches between 12 and 24 dB.
- **Tap the graph twice** where there is no dot to add a band there.
- Under the graph, tap a band's chip to pick it, **+ Band** to add one, and a type to change it: **Low cut**, **Low shelf**, **Bell**, **Notch**, **High shelf** or **High cut**. **ON** switches the band off and on again (it keeps its settings), **Remove** takes it away, and **Listen** plays only what that band touches, to help you find a ring or a boom; Listen follows the band you pick and stops when you leave the page.
- The Q-Links turn the picked band's frequency, gain and Q, and the EQ's output.

The Mix tab's **EQ** look shows every strip's EQ at once (see [Mixing](mixing-and-effects.md#every-eq-at-once-eq)).

### Pitch correct and the Harmonizer

Pitch correct and the Harmonizer listen for one voice: a singer, or a solo instrument such as a whistle or a sax, not chords. Set **Range** to fit it: **Low** for bass and baritone voices, **Mid** for most singing, **High** for high voices.

They hold the sound back a little while they listen (about 40 ms on Low, 28 on Mid and 17 on High), and the mixer holds everything else back to match, so the mix stays in time. When you sing live through one, High has the shortest delay.

### Master safety

The master strip always ends in **Master safety**, a limiter that keeps the output from going past -0.3 dB. It isn't in the list: it is always there. See [The master and Master safety](mixing-and-effects.md#the-master-and-master-safety).

## Effects with modes

**Reverb**, **Delay**, **Drive**, **Filter** and **Tremolo** each show once in the list of effects, and come in several modes. Hold one (or tap its presets line) to see every mode's presets, with the mode under each name; a preset adds the effect in its mode. A tap adds it in its first mode (a Room, a Digital delay, a Soft drive).

In the chain editor the **mode chooser** sits beside the effect's name, with the mode it is in lit: tap a mode to switch to it. Drive has too many to fit, so it shows the one it is in, with **‹** and **›** to step to the mode before or after: tap the mode's name for the list of all ten, and tap one. When you switch:

- The knobs the two modes share stay where you set them (as far as the new mode's range goes), so a Room switched to a Plate keeps its Decay, Size and Mix.
- Switch back and the first mode is as you left it, until you turn the MPC off.
- A macro or an LFO on a knob moves to the new mode's knob of the same name, and so does a Q-Link or a MIDI control you learned to it. One on a knob the new mode doesn't have comes off (learn it again on another knob), as do the effect's step locks.
- The effect keeps its place in the chain, and stays bypassed if it was.

To have two at once, add the effect twice and pick each one's mode: a Room, then a Plate, one after the other in the chain.

> [!NOTE]
> Projects made before modes open as they were: a Cloud reverb set to its own Plate or Room shows as the **Cloud reverb**, in none of the modes.

## XY FX

**XY FX** plays one effect from two controls at once, **X** and **Y**: Q-Links 1 and 2 turn them, and you can automate or lock them on steps like any knob. Pick the effect with **Effect**:

| Effect | X | Y |
|---|---|---|
| **Filter** | Low-pass to the left, high-pass to the right, off in the middle | Resonance |
| **Echo** | The echo's time, in steps from 1/32 to 1/2 | Feedback |
| **Dub echo** | A tape echo's time; moving it bends the pitch | Feedback, up to a howl |
| **Wash** | How long the reverb lasts | How bright it is |
| **Crush** | Sample rate, lower to the right | Fewer bits towards the top |
| **Flanger**, **Phaser** | The sweep, moved by hand | Feedback |
| **Ring mod** | The tone it rings at | How much |
| **Gate** | How fast it chops, from 1/32 to 1/4 | How deep |

**Active** switches the effect in and out, fading in over **Attack** and out over **Release**. Switched out, the Filter, Crush, Flanger, Phaser, Ring mod and Gate go back to the dry sound, while the echoes and the wash ring on: switching out an echo is a throw. **Mix** sets how much you hear. Changing the effect crossfades, so it doesn't click.

> [!NOTE]
> **Not yet:** the XY FX touch pad on the screen. It comes with the effects' own pages: touch the pad to play the effect where your finger is, lift to let it go (an echo rings out), and turn on **Hold** to keep it playing. For now X and Y are its first two knobs and Q-Links 1 and 2. The Play page has an XY pad of its own, for the DJ filter and an echo (see [Playing live](playing-live.md#the-xy-pad)).

## Macros

**Macros** are knobs that move several settings at once (Wash, Grit and so on). There are eight. The **Q-LINK** button steps the Q-Links through **Macros 1-4**, **Macros 5-8** and back to the page's own knobs, on the Play page, Arrange and in the chain editor.

Hold a finger still on a macro's card for half a second to open the **macros panel**, where you can see all of them and set up what each one moves. Back returns to the page you were on.

{{shot: macros | The macros panel on Macros 1-4.}}

## The effects already in place

Some effects are in place when MPFree starts, and the Play page's Q-Links and touch controls play them:

- **A DJ filter** on the Drums and Loop tracks: the **Filter** Q-Link on the Play page.
- **A dotted-eighth delay** on send return A: the **Delay send** Q-Link.
- **Perform FX** on the master: the throws and the XY pad (see [Playing live](playing-live.md#throws)).

Each of these costs almost nothing until you use it.

## Plugins

MPFree can also use plugin effects built for the MPC One's ARM chip, of two kinds:

- **CLAP plugins**, `.clap` files.
- **MPC plugins**, the kind Akai's own MPC software loads from a `VST2 Plugins` folder, such as the free plugins in the [Open MPC catalog](https://open-mpc.github.io). Each one is a folder named like `Airwindows - VST - Density2`.

Plugins made for a computer (Windows, macOS or ordinary Linux PCs) won't run on it.

1. For an MPC plugin from the Open MPC catalog, the easiest way is the **Plugins** place in BROWSE, which downloads and installs it for you, or installs a zip you copied to the stick (see [Getting plugins](plugins.md)).
2. Or put the plugins on the MPFREE-DATA partition of your MPFree stick yourself, in the folder `files/plugins/`, and start MPFree. For an MPC plugin, copy its whole folder there. These work too: the folder you unzipped from a catalog download (MPFree looks inside its `portable` folder), or a `VST2 Plugins` folder copied from an MPC drive.
3. The plugins' effects join the list you add effects from, under **Plugins** after the built-in ones, each with its maker's name.
4. Add one to a strip as you would any effect. Its knobs show in the chain editor, in the plugin's own groups, and the Q-Links, macros and MIDI Learn work on them.

Each plugin runs on its own, apart from the rest of MPFree, so a plugin that crashes or freezes can't stop the music:

- If a plugin is too slow for a moment, its strip plays dry (the sound goes straight through) for that moment.
- If a plugin crashes, MPFree starts it again with the settings it had a few seconds before.
- If it crashes four times in a minute, MPFree leaves it off and the strip plays dry. Remove it and add it again to try once more.

Each of these is written in the Events log, so nothing goes wrong without your being told.

Some plugins keep files of their own: Surge XT its user folder, Dexed its cartridges. MPFree gives them the folder `files/plugin-home/` on the stick, so they never write anywhere else. To reset every plugin's own files, delete that folder from a computer.

An MPC plugin effect's own screen (its skin) isn't drawn in the chain editor yet: its knobs show as a plain list of controls, in the order the plugin gives them. An MPC plugin instrument's skin shows on the Synth tab (see [Synths](synths.md#the-plugins-own-screen)). A few kinds of MPC plugin can't work in MPFree: ones made only for the newer 64-bit MPCs, add-ins that change Akai's own app, and ones that rely on parts of Akai's app. A sequencer plugin that sends notes out to other tracks loads, but its notes don't go anywhere yet.

A plugin's settings are saved with your project. If you open the project on a stick without that plugin, the effect is left out and the Events log names the plugin's file. Plugin synths play as a track's own instrument (see [Synths](synths.md#a-synth-for-each-track)).

> [!NOTE]
> **Not yet:** MPFree doesn't come with any plugins yet. A pack of free plugins built for the MPC (Airwindows, Surge XT and Dexed) is coming. MPC plugins are never included: you choose the ones you want, in [Getting plugins](plugins.md).

## Coming next

- **Automation**: knob moves recorded into clips.
- **Perform FX from the pads**: effects played by hitting pads.

> [!MPC]
> MPFree's effects open in the chain editor over the mixer rather than in a separate insert window. Effects with a delay of their own, such as Pitch correct, are lined up with the rest of the mix automatically, so nothing drifts out of time.
>
> Every MPC effect has an MPFree effect or preset in its place: the MPC's many Delay versions (Mono, Sync, Ping Pong, Analog, LP, HP) are the **Delay** and its presets, Delay Tape is the **Delay** in its **Tape** mode, Reverb Small, Medium and In Gate are the **Reverb** in its **Room** mode, Reverb Large and AIR Reverb its **Hall**, AIR Spring Reverb its **Spring**, Distortion Amp the **Amp**, AIR Enhancer the **Exciter**, AIR Maximizer and AIR Limiter the **Limiter**, AIR Noise Gate the **Gate**, AIR Pumper and Mother Ducker the **Ducker**, Decimator and Resampler **Crush**, the AIR Vocal Tuner **Pitch correct**, the AIR Vocal Harmonizer the **Harmonizer**, XYFX is **XY FX**, Auto Wah the **Auto wah**, the Frequency Shifter and AIR Freq Shift the **Freq shifter**, AIR Stereo Width the **Stereo width**, AIR Talkbox the **Talk box**, AIR Channel Strip the **Channel strip**, Granulator the **Granulator**, and Sample Delay the **Sample delay**.
