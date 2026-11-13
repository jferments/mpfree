<!--
status: changing
covers: rust/mpfree-synth rust/mpfree-synth-ui rust/mpfree-scale rust/mpfree-app/src/synth.rs rust/mpfree-app/src/app/synth_pads.rs rust/mpfree-app/src/instruments.rs
checked: 2026-11-13 9560b32
-->

# Synths

MPFree has five synths built in. Put one on the pads and they become a keyboard: bass lines, chords, leads and pads, played by hand, through the arpeggiator or into a pattern.

## Put a synth on the pads

There are three ways:

- Give a track its own synth: see [A synth for each track](#a-synth-for-each-track). The pads play it whenever that track is selected, and so do the track's clips.
- Tap the **Synth** tab. The pads play the synth while you are there and after you leave, until you pick a kit.
- Or, on the Play page, turn the data wheel past the last kit. The list ends with **Synth: Poly** (or the synth it had before tracks got their own).

To go back to drums, pick a kit with the data wheel on the Play page, or set the track's instrument back to **Kit**.

## The five synths

Any of the five can be a track's own instrument (see [A synth for each track](#a-synth-for-each-track)), and any track can have a different one.

| Synth | What it is | Good for | Presets |
|---|---|---|---|
| **Poly** | A classic two-oscillator subtractive synth, with a sub oscillator, noise, unison and a resonant filter | Basses, leads, pads, brass | Grit Bass, Acid, Supersaw, Dust Pad, Pluck, Brass, Sub Bass, Lead |
| **FM** | Six-operator FM, in the tradition of the 1980s digital synths | Electric pianos, bells, glassy keys, punchy basses | Glass Keys, Bell Tower, Rubber Bass, Drawbars, Brass Section, Log Drum, Silk Pad, Steel Pluck |
| **Drums** | A drum synth: a different synthesised drum on each pad | Electronic kits built from scratch | Boom Kit, Tight Kit, Grit Kit, Electro Kit |
| **Pluck** | Physically modelled strings and mallets | Guitars, harps, marimbas, bells | Nylon, Steel String, Harp, Koto, Muted Bass, Marimba, Vibes, Bell, Glass, Kalimba, Steel Drum |
| **Wavetable** | Two oscillators that sweep through stacks of waveforms | Evolving pads, vocal leads, growling basses | Morph Pad, Vowel Lead, Sync Sweep, Digital Bell, Growl Bass, Organ, Harmonic Pluck, FM Keys |

Each synth also has an **Init** preset (Init Kit on Drums): a plain starting point to build your own sound.

## A synth for each track

Every track can play its own instrument: Poly on one, FM on another, a plugin synth on a third, while the rest play the kit. The selected track is the one on the **Armed** row of the Record panel (see [Clips and patterns](clips.md#recording-a-clip)).

1. Select the track.
2. Press **MENU** on the Synth tab. The top row is named after the track: **Track 2 instrument**.
3. Change it. It steps through **Kit** (the pads' kit, as every track starts), **Poly**, **FM**, **Drums**, **Pluck** and **Wavetable**, then any plugin synths you have put in `files/plugins/` on the stick.

{{shot: synth-instrument | Track 2's instrument set to a plugin synth, with the row's help on the right.}}

- **It starts as a copy** of the synth's sound on the Synth tab, edits included. From then on it is the track's own: the Synth tab edits the selected track's instrument, and its title says whose it is ("Track 2"). Loading a preset gives the track a fresh copy with that preset.
- **The pads play the selected track.** Select another track and the pads play its instrument, or the kit if it has none. The track's clips play its instrument too, whichever track is selected.
- **Changing it is smooth.** Notes still sounding on the old instrument finish naturally while the new one starts. **Kit** takes the instrument away, and the track plays the kit again.
- **Its own mixer channel.** A track with an instrument gets a channel named after it (**Track 1**, **Track 2**) in the mixer's Tracks view, with its own fader, inserts and sends. The channel only shows while the track has an instrument. See [Mixing and effects](mixing-and-effects.md).
- **Shared options.** The scale, pad layouts, tuning and octave on the Synth tab's options are the same for every track's instrument.
- **A plugin synth** runs in a process of its own, as plugin effects do. If it crashes, the track goes quiet and the plugin restarts on its own, while the rest of MPFree plays on. If it can't be started at all, the line along the bottom of the screen says so for a few seconds and the track keeps what it had. Its controls show on the Synth tab like a built-in synth's, without presets.

{{shot: synth-track | The Synth tab editing track 2's own FM, with the Glass Keys preset.}}

- **Saved with the project.** Each track's instrument, with its sound (or a plugin synth's settings), is part of the project. Opening a project puts every track's instrument back, and **New project** puts every track on the kit (see [Saving your work](saving.md#projects)).

On Arrange, each track's instrument is written under its name, on the clip grid and in the Song view. Tap that line on the clip grid to go straight to the track's instrument row (see [Clips and patterns](clips.md#tracks-and-slots)).

Coming:
- The instrument's name beside each track on the Play page.
- The voices and meters on the Synth tab's Output page show the pads' synth, not a track's own.

## The Synth tab

{{shot: synth | The Synth tab with Poly playing its Pluck preset. Pictures of the oscillators, filter, envelope and LFO across the top, the Osc page's controls below, and the presets on the right.}}

- **The title** shows the synth and the preset. The tabs beside it are the synth's **pages**: for Poly, Osc, Filter, Env, LFO, Voice and Pressure. Tap a tab, or hold **SHIFT** and turn the data wheel, to change page.
- **The pictures** across the top draw the sound as you change it: the oscillator waves, the filter's curve, the envelopes and the LFO. Tap one to jump to its page.
- **The tiles** are the controls on this page. Drag a tile up or down to change it. Or pick a tile with the data wheel (or a tap) and press **–** or **+**; push the wheel and turning it changes the tile picked (outlined more strongly) until you push it again.
- **The Q-Links** turn the four tiles marked Q1 to Q4. Each preset chooses which four, so the knobs always do something useful for that sound. Each Q-Link has its own colour, the same on its tile's mark and knob and in the Q-Link row along the bottom.
- **The presets** are listed on the right. Tap one to load it. Once you change anything, the preset is marked **edited**.

**Drive**, on the Filter page of Poly and Wavetable, saturates the sound before the filter. It keeps the sound at about the same level as you turn it up (within about 2 dB on every preset), so it adds grit and thickness, not volume; turn up the synth's level for that.

Each synth has its own pages. FM has Algo, Mod and Pressure, a page for each of its six operators and three keyboard-scaling pages; Drums has a page for each of its 16 pads plus a Kit page.

The Synth tab takes its colours from the theme picked in **Prefs > Appearance**, like the rest of MPFree: the Q-Link colours, the curves and the boxes in the pictures all change with it.

{{shot: synth-fm | The FM synth: the algorithm diagram shows which operators modulate which, and the levels show the six operators.}}

> [!NOTE]
> Each synth's sound, edits and all, is saved with the project (see [Saving your work](saving.md#projects)). Saving an edited sound as a preset of its own comes later.

## Playing the synth on the pads

With no scale, the pads are laid out like a keyboard: pad 1 is C3 and each pad after it is a semitone higher, up to D#4 on pad 16. On the Play page, hold **SHIFT** and turn the data wheel to move the whole pad range up or down an octave at a time (from C0 to C6 at pad 1).

Choose a scale and the pads can help you stay in key. Press **MENU** on the Synth tab:

| Setting | What it does |
|---|---|
| **Scale** | The pads' scale, or Off. Nearly 50 to choose from, as for the arpeggiator. |
| **Scale root** | The scale's first note. |
| **Pad layout** | How the pads use the scale (below). Each pad bank keeps its own. |
| **In key rows** | For In key: whether the pads climb the scale in sequence, or each row starts a fourth above the one below, so a chord shape is the same anywhere on the pads. |

There are three pad layouts:

- **Chromatic**: every semitone, as without a scale, but the pads are lit by the scale: the root, the other notes in the scale, and the notes outside it in a dimmer colour.
- **In key**: only the scale's notes, starting on the root. Every pad is a "right" note.
- **Chord pads**: each pad plays a whole chord from the scale: triads on the bottom rows, sevenths on the top rows.

With a scale on, the four pad banks start out as four different layouts, so you can switch between them with the **PAD BANK** buttons as you play: **A** Chromatic, **B** In key, **C** Chord pads, **D** In key an octave higher.

The **Chromatic / In key** switch in the pads heading on the Synth tab flips the current bank between the two with one tap. With no scale picked yet, **In key** picks **Major** on the Scale root (C to start), so the pads change at once; the line at the bottom of the screen says which scale they play. Change the scale in the Synth tab's options (**MENU**, **Scale**).

{{shot: play-synth-in-key | The Poly synth in A minor, In key, on the Play page. Every pad plays a note of the scale, and the pattern's rows are named by note.}}

> [!TIP]
> Drums is a kit rather than a keyboard: pad 1 plays the first drum, pad 2 the second and so on, whatever the scale, layout or octave.

## Tuning

Out of the box every synth plays in standard tuning: twelve equal steps to the octave, with the A above middle C at 440 Hz. You can change that for the synth on the pads in the Synth tab's options (press MENU on the Synth tab; see [Controls](../reference/controls.md)).

{{shot: synth-options | The Synth tab's options, with the Tuning, Tuning root and A4 rows.}}

- **Tuning** picks how the notes are tuned:
  - **12-TET** is standard tuning, the same in every key.
  - **Just (5-limit)** makes thirds and fifths beat-free and sweet, in keys near its root.
  - **Pythagorean** has pure fifths, for drones and open, medieval-sounding chords.
  - **Meantone 1/4 comma** (quarter-comma meantone) has pure major thirds, as on Renaissance and early Baroque keyboards.
  - **24-TET quarter tones** has 24 equal steps to the octave, so each step up is half a semitone.
  - After those come your own tuning files, by file name (see below).
- **Tuning root** is the note the tuning is built on. Just, Pythagorean, meantone and your own files sound purest in keys near their root, so set it to your song's key. The two equal tunings sound the same in every key and show "C (not used)".
- **A4** is the pitch of the A above middle C, from 380 to 480 Hz, so the synth can match a recording or another instrument. 432 Hz and 442 Hz are common choices.

The drum synth keeps its own pitches whatever you pick here.

Everything else in the Synth tab's options (the scale, pad layouts and tuning) is the song's: it is saved in your project and comes back when you open it, and New project starts chromatic in C at A4 = 440 Hz (see [Saving your work](saving.md)). If a tuning file or one of your own scales is gone by then, that one choice goes back to its default, and the Console says so.

### Your own tunings

MPFree reads Scala tuning files, the format thousands of published tunings come in. Put them on the MPFREE-DATA partition of your MPFree stick, in the folder `files/tunings/`, and start MPFree. They join the Tuning row after the built-in tunings, in name order, up to 64.

- **A scale** is a `.scl` file. Each step up the keyboard (each pad up, with the Chromatic pad layout) plays the next note of the scale, starting from the Tuning root, and the root keeps its usual pitch for the A4 you pick, just as with the built-in tunings. A scale needn't have 12 notes: a 19-note or a 7-note scale works, and so does one that repeats at something other than the octave.
- **A keyboard map** is a `.kbm` file with the same name beside the scale (`dorian.scl` and `dorian.kbm`). It says which key plays which note of the scale and what pitch the tuning sits at, so with one the Tuning root and A4 rows show "set by its .kbm".
- **Names** must be lower case: letters, digits, `.`, `_` and `-` only, such as `werckmeister3.scl`. A file named any other way isn't seen. The row shows at most 20 characters of a name.
- MPFree only reads the folder; it never changes your files. A file it can't read is left out, and the Console (see [Controls](../reference/controls.md)) says which and why, and lists the files it found.

## Pad pressure

The MPC's pads sense how hard you keep pressing after the hit. Each synth's **Pressure** page sets what that does. By default on Poly, pressing harder opens the filter, so a held chord brightens as you lean into it. You choose up to four things for pressure to change, how much, and how hard you must press before it starts.

Clips record pad pressure too, so a recorded part plays back with the same swells: see [Pad pressure](clips.md#pad-pressure) in Clips and patterns.

## Velocity

How hard you hit a pad sets how loud the note plays, as with drums. **FULL LEVEL** and Half Level, and the velocity curve in [Prefs](../reference/prefs.md), work the same for the synth.

Note Repeat, Latch and the arpeggiator all play the synth too: see [Note Repeat, Latch and the arpeggiator](note-repeat-and-arp.md).

> [!MPC]
> The MPC's plugin instruments (such as its bass and electric-piano plugins) and expansions are not in MPFree. MPFree has its own five synths instead, with their controls on one tab and the Q-Links following each preset, and it can host CLAP plugin synths. As on the MPC, where each track plays a program, each track can play its own instrument; a track without one plays the kit on the pads. The scales and chords of the MPC's Pad Perform mode are replaced by the scale settings and the per-bank pad layouts here: switch layouts with the PAD BANK buttons without leaving the page you are on.
