<!--
status: changing
covers: rust/mpfree-arp rust/mpfree-arp-ui rust/mpfree-scale rust/mpfree-app/src/app/arp.rs
checked: 2026-11-12 ae9c39c
-->

# Note Repeat, Latch and the arpeggiator

Hold **NOTE REPEAT** and a pad you hold plays again and again, exactly in time: hi-hat rolls, snare fills, rapid-fire chops. Latch keeps it going after you let go, so your hands are free. And the same engine turns held notes into arpeggios and rhythmic patterns, which is how a single held chord becomes a moving synth line.

All of this works on drum kits and on the synth alike. Everything it plays is in time with the transport, and when the transport is stopped it keeps its own time at the current tempo.

## Note Repeat

1. Hold **NOTE REPEAT**.
2. Hold a pad. It repeats at the **repeat rate**, 1/16 notes to begin with.
3. Hit the pad harder or softer as you hold it: each repeat plays at the pressure you are pressing with now, so you can swell a roll up or fade it out.
4. Let go of NOTE REPEAT and the pads play normally again.

While NOTE REPEAT is held, a slim **quick band** appears on the Play page with the mode and the rate, and the controls change job:

| Control | While NOTE REPEAT is held |
|---|---|
| Data wheel | The rate: 1/4, 1/8, 1/8T, 1/16, 1/16T, 1/32 or 1/32T (T is a triplet) |
| **–** and **+** | The mode: Repeat, Arp or Pattern (see below) |
| Q-Link 1 | Rate |
| Q-Link 2 | Gate: how long each repeat sounds, as a share of the step |
| Q-Link 3 | Swing |
| Q-Link 4 | Octaves: how many octaves an arpeggio climbs |

{{shot: arp-quick-band | Holding NOTE REPEAT: the quick band shows the mode (Repeat) and the rate, with – and + to change it and a LATCH button. The Q-Links turn rate, gate, swing and octaves.}}

You can also set the rate without holding NOTE REPEAT: press **MENU** on the Play page and change **Note repeat rate**.

## Latch

**Latch** keeps the pads you play repeating after you let go of them and of NOTE REPEAT. Press NOTE REPEAT twice quickly to turn it on (or tap **LATCH** on the quick band, or see [Buttons and controls](../reference/controls.md#repeat-and-arpeggiator) for the other way). Press NOTE REPEAT once to turn it off; **STOP** also ends it.

What a new pad press does while Latch is on is a setting in [Prefs](../reference/prefs.md), **Latch keeps pads**:

- **Replace** (the default): a fresh press after you let go of everything starts a new set of latched pads.
- **Toggle**: each press adds that pad to the set or takes it out, so you can build a groove pad by pad.

Latched pads stay lit on the MPC, and the top bar says so.

For a sound that should simply keep playing after you let go (a loop, a drone, a grain cloud) without repeating, give the pad its own **Latch** in the Sample tab instead: see [Pads that stop, play or hold others](sampling.md#pads-that-stop-play-or-hold-others).

> [!TIP]
> Latch a hi-hat at 1/16, then play the kick and snare over it by hand. With the pattern recording, the latched hats record too.

## The Arp panel

The **Arp panel** opens on the Play page, in place of the view under the transport bar, with **NOTE REPEAT + MENU**, or by tapping the arp chip in the top bar. The same again closes it. While it is open, the pads play through the arp in the mode its tabs pick, and you don't need to hold NOTE REPEAT.

{{shot: arp-panel | The Arp panel in Arp mode: the order (Up), the four Q-Link settings as sliders, and the notes being played along the bottom, here 1-3-5 and the same an octave up.}}

The four tabs along the top are the four **modes**:

| Mode | What the pads do |
|---|---|
| **Direct** | Play as pressed. The arp is out of the way. |
| **Repeat** | Each held pad repeats at the rate (this is Note Repeat). |
| **Arp** | The held notes play one after another, in an order, over the octaves you set. |
| **Pattern** | The held notes follow a rhythmic pattern of steps, with rests, ties, accents and octave jumps. |

Along the bottom of the panel, the notes the arp is about to play light up in turn, with their octave.

### Orders

In Arp mode the data wheel picks the **order** the held notes play in:

| Order | Plays |
|---|---|
| Up, Down | Lowest to highest, or highest to lowest |
| Up-down | Up then down, without repeating the top and bottom notes |
| Up-down incl. | Up then down, playing the top and bottom notes twice |
| Converge, Diverge | From the outside in, or from the middle out |
| As played | In the order you pressed the pads |
| Random | Any held note each step |
| Random, no repeats | Any note but the one just played |
| Random once | A random order, then that same order over and over |
| Chord | All the held notes together on every step |

### Patterns

In Pattern mode the wheel picks a **pattern**. MPFree comes with ten:

| Pattern | Feel |
|---|---|
| Straight | Every step, the held notes in turn |
| Gallop | Two hits, a rest, a hit |
| Bounce | First, third, second, third note |
| Climb | Three notes, then the same an octave up and two up |
| Pulse | Accents and soft notes in a pumping pulse |
| Stabs | Chord stabs with rests between |
| Rolls | Single hits, then quick rolls of two and three |
| Drift | Loose: random notes, chance steps and ties |
| Three over four | A three-note octave figure across a four-step bar, with its own accents |
| Climbing line | One held note becomes a scale line going up and back |

Under the list, four buttons work on the pattern:

- **Edit pattern** opens the pattern workspace (below).
- **Record on pads** opens the workspace and records a new pattern from the pads.
- **Mutate** changes some of the pattern's steps at random. Press it again for more.
- **Original** puts back the pattern as it was in the list.

## Arp options

Press **MENU** while the Arp panel is open for the arp's own settings. Each one has a short explanation beside it on the screen.

| Setting | What it does |
|---|---|
| **Tempo** | The song's tempo. |
| **Accent level** | How loud accented steps play. Hits of 110 or more record as accents. |
| **Scale** and **Scale root** | Keep the arp's notes in a key (see the next section). |
| **Out-of-key notes** | With a scale on, what a held note outside the scale plays: the scale note below, above, the nearer one, or nothing. |
| **Chord on each pad** | Each pad you hold plays a whole chord built from the scale: Power, Triad, Sus2, Sus4, Seventh or Ninth. One pad is enough for an arpeggio. |
| **Scale run** | The arp plays every scale note between your lowest and highest held note, not just the ones you hold. |
| **Chords take turns** | Hold two or more pads and they take turns (every beat, two beats, bar or two bars) instead of sounding together. Hold four chords and the arp plays a chord progression. |
| **Strum** | Spreads the notes of a chord step across the step, lowest first, like a strummed guitar. |
| **New notes restart** | On: every new pad press starts the order and the pattern again from the first step. Off: they carry on. |
| **Swing from** | **Transport**: while the sequencer plays with swing, the arp swings with it. **Arp**: always the arp's own swing (Q-Link 3). |

## Scales and chords

MPFree has nearly 50 scales, from Major and Minor and the church modes through harmonic and melodic minor, pentatonics and blues, to bebop scales and Japanese scales such as Hirajoshi and In-sen. Pick one with **Scale** and its first note with **Scale root**.

There is no single key for the whole song unless you choose one. The arp has its own scale, and the synth's pads have their own (see [Synths](synths.md)), so you can play the pads chromatically while the arp stays in key, or the other way round.

Scales are for the synth. Leave the arp's scale **Off** for a drum kit: its pads are different sounds, not notes, and a scale would move a snare onto a clap.

> [!TIP]
> For an instant chord progression: set Scale to Minor, Chord on each pad to Seventh, Chords take turns to Every bar, then hold four pads. Each bar plays an arpeggio of the next chord.

## The pattern workspace

**Edit pattern** or **Record on pads** on the Arp panel opens the **pattern workspace**, which shows a pattern step by step. Each column is a step, and each row one of its settings:

| Row | What it sets |
|---|---|
| **Step** | Play, rest, tie (hold the previous note on) or random |
| **Note** | Which held note plays (1 is the lowest) |
| **Octave** | Up or down from that note |
| **Ratchet** | Plays the step as a quick roll of 2, 3 or more hits |
| **Chance** | How likely the step is to play |
| **Gate** | How long the step sounds |

Drag a cell up or down, or select it and turn the data wheel, to change it. **Rotate ←** and **Rotate →** shift the pattern a step; **Mutate** changes it at random; **Length −** and **Length +** shorten or lengthen it; **Original** puts it back; **Save** keeps it; **Back** returns to the Arp panel.

{{shot: arp-workspace-record | Recording a pattern on the pads. Pads 1 to 9 are the held notes; 13 to 16 add a next step, a random step, a tie or a rest.}}

### Recording a pattern on the pads

1. Tap **Record on pads**. The pads change job: pads 1 to 9 are the 1st to 9th held note, and the top row adds a **next** step, a **random** note, a **tie** and a **rest**.
2. Tap the steps in. In **Step time** each press writes the next step, at your own pace. In **Free time** your timing is kept.
3. Hit a pad hard (110 or more) for an accented step.
4. Tap **Stop recording** when you are done, or recording stops by itself when the pattern is full. **Undo** takes back the last step and **Clear** starts again.
5. Tap **Save**.

> [!NOTE]
> Saved patterns are kept on the MPFree stick, so they are still there after you switch off. Each track's arp, with its pattern as you left it, is saved in your project too, and comes back when you open it. See [Saving your work](saving.md).

> [!MPC]
> On the MPC, Note Repeat and the Arpeggiator are separate, and the arpeggiator is a full-screen editor. In MPFree they are one engine: NOTE REPEAT's – and + switch between repeat, arp and pattern while you play, and the Arp panel takes the Play page's view while the transport bar, the master bar and the Q-Links stay on screen. Latch is a double press of NOTE REPEAT. MPFree's patterns (steps with ratchets, chance, ties and octave jumps) have no equivalent on the MPC.
