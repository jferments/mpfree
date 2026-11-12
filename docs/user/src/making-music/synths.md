<!--
status: changing
covers: rust/mpfree-synth rust/mpfree-synth-ui rust/mpfree-scale rust/mpfree-app/src/synth.rs rust/mpfree-app/src/app/synth_pads.rs
checked: 2026-11-12 2469645
-->

# Synths

MPFree has five synths built in. Put one on the pads and they become a keyboard: bass lines, chords, leads and pads, played by hand, through the arpeggiator or into a pattern.

## Put a synth on the pads

There are two ways:

- Tap the **Synth** tab. The pads play the synth while you are there and after you leave, until you pick a kit.
- Or, on the Play page, turn the data wheel past the last kit. The list ends with **Synth: Poly** (or whichever synth you picked last).

To go back to drums, pick a kit with the data wheel on the Play page.

## The five synths

Press **MENU** on the Synth tab and change **Engine** to switch synth. A note still sounding on the old one finishes naturally.

| Synth | What it is | Good for | Presets |
|---|---|---|---|
| **Poly** | A classic two-oscillator subtractive synth, with a sub oscillator, noise, unison and a resonant filter | Basses, leads, pads, brass | Grit Bass, Acid, Supersaw, Dust Pad, Pluck, Brass, Sub Bass, Lead |
| **FM** | Six-operator FM, in the tradition of the 1980s digital synths | Electric pianos, bells, glassy keys, punchy basses | Glass Keys, Bell Tower, Rubber Bass, Drawbars, Brass Section, Log Drum, Silk Pad, Steel Pluck |
| **Drums** | A drum synth: a different synthesised drum on each pad | Electronic kits built from scratch | Boom Kit, Tight Kit, Grit Kit, Electro Kit |
| **Pluck** | Physically modelled strings and mallets | Guitars, harps, marimbas, bells | Nylon, Steel String, Harp, Koto, Muted Bass, Marimba, Vibes, Bell, Glass, Kalimba, Steel Drum |
| **Wavetable** | Two oscillators that sweep through stacks of waveforms | Evolving pads, vocal leads, growling basses | Morph Pad, Vowel Lead, Sync Sweep, Digital Bell, Growl Bass, Organ, Harmonic Pluck, FM Keys |

Each synth also has an **Init** preset (Init Kit on Drums): a plain starting point to build your own sound.

## The Synth tab

{{shot: synth | The Synth tab with Poly playing its Pluck preset. Pictures of the oscillators, filter, envelope and LFO across the top, the Osc page's controls below, and the presets on the right.}}

- **The title** shows the synth and the preset. The tabs beside it are the synth's **pages**: for Poly, Osc, Filter, Env, LFO, Voice and Pressure. Tap a tab, or press **–** and **+**, to change page.
- **The pictures** across the top draw the sound as you change it: the oscillator waves, the filter's curve, the envelopes and the LFO. Tap one to jump to its page.
- **The tiles** are the controls on this page. Drag a tile up or down to change it, or tap it and turn the data wheel.
- **The Q-Links** turn the four tiles marked Q1 to Q4. Each preset chooses which four, so the knobs always do something useful for that sound.
- **The presets** are listed on the right. Tap one to load it. Once you change anything, the preset is marked **edited**.

Each synth has its own pages. FM has Algo, Mod and Pressure, a page for each of its six operators and three keyboard-scaling pages; Drums has a page for each of its 16 pads plus a Kit page.

{{shot: synth-fm | The FM synth: the algorithm diagram shows which operators modulate which, and the levels show the six operators.}}

> [!NOTE]
> Edits to a preset last until you load another preset or switch off. Saving your own presets comes with the browser (see [Saving your work](saving.md)).

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

The **Chromatic / In key** switch at the top of the pad map, on the Play page and the Synth tab, flips the current bank between the two with one tap.

{{shot: play-synth-in-key | The Poly synth in A minor, In key. Every pad plays a note of the scale, and the pattern's rows are named by note.}}

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
- **Tuning root** is the note the tuning is built on. Just, Pythagorean and meantone sound purest in keys near their root, so set it to your song's key. The two equal tunings sound the same in every key and show "C (not used)".
- **A4** is the pitch of the A above middle C, from 380 to 480 Hz, so the synth can match a recording or another instrument. 432 Hz and 442 Hz are common choices.

The drum synth keeps its own pitches whatever you pick here.

## Pad pressure

The MPC's pads sense how hard you keep pressing after the hit. Each synth's **Pressure** page sets what that does. By default on Poly, pressing harder opens the filter, so a held chord brightens as you lean into it. You choose up to four things for pressure to change, how much, and how hard you must press before it starts.

## Velocity

How hard you hit a pad sets how loud the note plays, as with drums. **FULL LEVEL** and Half Level, and the velocity curve in [Prefs](../reference/prefs.md), work the same for the synth.

Note Repeat, Latch and the arpeggiator all play the synth too: see [Note Repeat, Latch and the arpeggiator](note-repeat-and-arp.md).

> [!MPC]
> The MPC's plugin instruments (such as its bass and electric-piano plugins) and expansions are not in MPFree. MPFree has its own five synths instead, with their controls on one tab and the Q-Links following each preset. The scales and chords of the MPC's Pad Perform mode are replaced by the scale settings and the per-bank pad layouts here: switch layouts with the PAD BANK buttons without leaving the page you are on.
