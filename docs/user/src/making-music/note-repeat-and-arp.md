<!--
status: changing
covers: rust/mpfree-arp rust/mpfree-arp-ui rust/mpfree-scale rust/mpfree-app/src/app/arp.rs
checked: 2026-11-19 a6fb29b2
-->

# Note Repeat, Latch and the arpeggiator

Hold **NOTE REPEAT** and a pad you hold plays again and again, exactly in time: hi-hat rolls, snare fills, rapid-fire chops. Latch keeps it going after you let go, so your hands are free. And the same engine turns held notes into arpeggios and rhythmic patterns, which is how a single held chord becomes a moving synth line. On a synth it plays **phrases** too: a whole bass line or riff from one held note.

Note Repeat, Latch, the arp and patterns work on drum kits and on the synth alike; phrases move notes, so they play on synth tracks. Everything it plays is in time with the transport, and when the transport is stopped it keeps its own time at the current tempo.

## Note Repeat

1. Hold **NOTE REPEAT**.
2. Hold a pad. It repeats at the **repeat rate**, 1/16 notes to begin with.
3. Hit the pad harder or softer as you hold it: each repeat plays at the pressure you are pressing with now, so you can swell a roll up or fade it out.
4. Let go of NOTE REPEAT and the pads play normally again.

While NOTE REPEAT is held, a slim **quick band** appears on the Play page, between the loops and the rows, with the mode, the rate and **LATCH**, and the controls change job:

| Control | While NOTE REPEAT is held |
|---|---|
| Data wheel | The rate: 1/4, 1/8, 1/8T, 1/16, 1/16T, 1/32 or 1/32T (T is a triplet) |
| **–** and **+** | The mode: Repeat, Arp or Pattern, and on a synth track Phrase (see below) |
| Q-Link 1 | Rate (while a phrase plays, its speed) |
| Q-Link 2 | Gate: how long each repeat sounds, as a share of the step |
| Q-Link 3 | Swing |
| Q-Link 4 | Octaves: how many octaves an arpeggio climbs |

{{shot: arp-quick-band | Holding NOTE REPEAT: the quick band shows the mode (Repeat) and the rate, with – and + to change it and a LATCH button. The Q-Links turn rate, gate, swing and octaves.}}

You can also set the rate without holding NOTE REPEAT: press **MENU** on the Play page and change **Note repeat rate**.

## Latch

**Latch** keeps the pads you play repeating after you let go of them and of NOTE REPEAT. Press NOTE REPEAT twice quickly to turn it on (or tap **LATCH** on the quick band, or see [Buttons and controls](../reference/controls.md#repeat-and-arpeggiator) for the other way). Press NOTE REPEAT once to turn it off; **STOP** also ends it.

What a new pad press does while Latch is on is a setting in [Prefs](../reference/prefs.md), **Latch keeps pads**:

- **Replace** (the default): a fresh press after you let go of everything starts a new set of latched pads. To latch several pads, hold them down together.
- **Toggle**: each press adds that pad to the set or takes it out, so you can build a groove pad by pad.
- **Add**: every press joins the set and stays until Latch goes, so you can build a chord or a groove one pad at a time without taking any out.

### Latching with the sustain pedal

With a keyboard's sustain pedal plugged in, turn on **Sustain pedal latches** (in [Prefs](../reference/prefs.md), or in the arp's options below) and the pedal works Latch while the arp plays (Repeat, Arp, Pattern or Phrase): hold the pedal down and what you play keeps going after you let go; lift it and the notes you aren't holding stop. While it latches the arp, the pedal doesn't hold the sound's notes. With the arp on Direct, the pedal sustains the sound as it always does.

A latched pad repeats at the velocity you struck it with, not at the pressure of your finger as it lifted.

Latched pads stay lit on the MPC, and the top bar says so.

For a sound that should simply keep playing after you let go (a loop, a drone, a grain cloud) without repeating, give the pad its own **Latch** in the Sample tab instead: see [Pads that stop, play or hold others](shaping-a-pad.md#pads-that-stop-play-or-hold-others).

> [!TIP]
> Latch a hi-hat at 1/16, then play the kick and snare over it by hand. With the pattern recording, the latched hats record too.

## When the pads play something of their own

Note Repeat, Latch and the arpeggiator play whatever the pads play, and turning them on never changes that. When you have chosen a synth or a kit for the pads (the top bar reads **Pads: Poly**, say) while the selected track plays something else, the repeats and arpeggios play that synth or kit, with the selected track's arp settings: its mode, rate, gate, swing, pattern and Latch. Your drums and the track's clips go on as before.

- **On a synth** the arp plays the notes of the pads you hold, chords too, as the synth's pad layout has them.
- **On a kit** it plays the kit's pads on the bank the pads are on.
- **When the pads move to something else** (Follow track, another bank with its own pick, another synth or kit), what the arp was playing there stops, latched notes too. Play again to start it on what the pads play now.
- **A phrase stays with its track.** A [phrase](#phrases) moves the notes of the selected track's synth, so the synth or kit the pads play on their own never plays one: with the Arp panel on Phrase the pads play it as pressed, and NOTE REPEAT and Latch repeat it.
- **Layers** play each hit once, as you press it; the repeats play the pads' first sound.
- **Recording:** REC records the synth or kit on a free track as usual (see [Synths](synths.md)), and the arp's notes record as you hear them. Once that track is selected the pads play through it and its arp. While the pads play a bank's own synth or kit, the arp's notes go into that sound's own track (see [Recording every bank](synths.md#recording-every-bank)).

## The Arp panel

The **Arp panel** opens on the Play page, in place of the loops, the rows and the throws, with **NOTE REPEAT + MENU**, or by tapping the arp chip in the top bar (such as **Arp: Up 1/16, latched · 120 BPM**, which shows while the arp, a repeat or a latch is on). The same again closes it. While it is open, the pads play through the arp in the mode its tabs pick, and you don't need to hold NOTE REPEAT.

{{shot: arp-panel | The Arp panel in Arp mode: the order (Up), the four Q-Link settings as sliders, and the notes being played along the bottom, here 1-3-5 and the same an octave up.}}

The five tabs along the top are the five **modes**:

| Mode | What the pads do |
|---|---|
| **Direct** | Play as pressed. The arp is out of the way. |
| **Repeat** | Each held pad repeats at the rate (this is Note Repeat). |
| **Arp** | The held notes play one after another, in an order, over the octaves you set. |
| **Pattern** | The held notes follow a rhythmic pattern of steps, with rests, ties, accents and octave jumps. |
| **Phrase** | One held note plays a whole phrase: a bass line, a riff, chords or an arp line. On synth tracks (see [Phrases](#phrases)). |

Along the bottom of the panel, the notes the arp is about to play light up in turn, with their octave. (The Phrase tab draws its phrase's notes in a picture of their own.)

To turn rate, gate, swing or octaves from a knob on your keyboard or controller, hold **SHIFT**, touch the slider, then move the knob (see [MIDI Learn](midi-and-cv.md#midi-learn)). The same works on the Q-Link cards while NOTE REPEAT is held or latched.

### Orders

In Arp mode the data wheel picks the **order** the held notes play in, from eleven:

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

In Pattern mode the list holds MPFree's **74 patterns**, in nine groups, and yours after them. The groups are tabs along the top of the list: tap one and its first pattern plays. The wheel picks a pattern, and **SHIFT + wheel** jumps to the next group (or back to the start of this one, then the one before).

{{shot: arp-pattern-rolls | The Rolls group, with Snare build picked: each row shows the rhythm of its first bar, and its steps as text at the right.}}

Each row has a picture of the pattern's first bar, so you can see its rhythm before you hear it: a block for each hit, a tall orange block for an accent, thin bars for a roll (several hits in one step), three small blocks for the whole chord at once, a faint block for a step that plays only sometimes, a bar for a tie and a dot for a rest. Patterns longer than a bar say how many steps they have.

| Group | What's in it |
|---|---|
| Starters | The first ten: straight steps, a gallop, bounces, climbs, stabs and rolls |
| Chords | The whole chord in rhythm: offbeats, two and four, tresillo, clave, Charleston, two-step, long chords, a funk comp |
| Gates | The chord chopped and pumped: a trance gate, a sidechain-style pump, stutters, swells |
| Rolls | Rolls for hats and snares: trap hats, a snare build, triplet rolls, a four-bar fill |
| Riffs | The low note does the work: gallops, chugs, palm mutes, a breakdown, tremolo |
| Lines | Melodic shapes over the chord: spirals, octave leaps, a waterfall, a pentatonic run |
| Bass | Bass lines from the lowest note: disco octaves, rolling, acid, sub, walking |
| Polymeter | Lanes and lengths that loop against the bar: five over four, Euclidean rhythms, dotted eighths |
| Chance | Never quite the same bar twice: sparse hits, ghost notes, random notes and rolls |
| My patterns | Yours: patterns from your patterns file that aren't MPFree's (a pattern you save in the workspace keeps its place in its group) |

Every pattern plays the notes you hold, so it follows your chord. [Arp patterns](../reference/arp-patterns.md) lists all 74 and what each one plays.

Under the list, four buttons work on the pattern:

- **Edit pattern** opens the pattern workspace (below).
- **Record on pads** opens the workspace and records a new pattern from the pads.
- **Mutate** changes some of the pattern's steps at random. Press it again for more.
- **Original** puts back the pattern as it was in the list.

### Phrases

A **phrase** is a short piece of music, a bar or two long, that plays from the note you hold: a bass line, a riff, a run of chords or an arp line, with its own rhythm, long and short notes and accents. Hold one pad on a synth track and the whole phrase plays from that pad's note, round and round. Hold another pad and it carries on from there, without losing its place. It is the quickest way from one finger to a part that sounds written.

{{shot: arp-phrase | The Phrase tab: the kinds of phrase on the left, the basses in the middle with Night drive picked, and on the right its notes over two bars with a line where it is playing, what it follows, and the four Q-Link settings.}}

MPFree comes with 64 phrases of its own, 16 of each kind: **Bass**, **Riff**, **Chords** and **Arp lines**. **Yours** is for phrases of your own. It is empty until you have one, and making them on the screen comes next.

1. Open the Arp panel on a synth track and tap **Phrase**.
2. Tap a kind on the left, then a phrase in the list, or turn the data wheel.
3. Hold a pad. Latch keeps the phrase going, as it does the arp.

| Control | On the Phrase tab |
|---|---|
| Data wheel | The phrase, up and down the kind that shows |
| **–** and **+** | The kind. **–** on the first kind goes back to the Pattern tab |
| Q-Link 1 | **Speed**: half, as written or double |
| Q-Link 2 | **Gate**: shorter or longer notes. Halfway plays them as written |
| Q-Link 3 | **Swing** |
| Q-Link 4 | **Octaves**: with 2 or more, each time through plays an octave higher than the last, then it starts from the bottom again |

Tapping a kind, or stepping to it with **–** and **+**, also picks a phrase of that kind: the one you last played there, or its first. So the row lit in the list is always the phrase a held pad plays, and stepping through the kinds is a quick way to hear one of each.

In the list, each phrase shows its length and its notes drawn small. The phrases that suit the key you are in come first. One written for the other kind of scale (a major phrase while you play in minor) comes after them, faint, and says which scale it was written in. It still plays, moved onto your key.

**Follows**, under the phrase's picture, sets how the phrase finds its notes:

- **In key** moves the phrase onto a scale, so every note belongs. It follows the arp's own **Scale** if you set one (see [Arp options](#arp-options)), otherwise the scale the pads play; the key is named beside the switch. With no scale anywhere it plays by semitones, as Exact does.
- **Exact** plays the phrase's notes as written, counted in semitones from the note you hold, whatever the key.

A phrase written in triplets is never swung: its Swing slider goes faint and reads "straight". Gate, Swing and Octaves learn a knob as on the other tabs; **Speed** can't learn one yet. The same goes for the [XY pad](xy-layouts.md#the-arp-on-the-pad): a layout of yours can hold a phrase's Gate, Swing and Octaves, but not its Speed yet. A layout that holds the arp's Rate plays a phrase faster or slower while your finger is down, and the phrase goes back to its own speed when you lift, even with Let go: Stay.

#### Changing phrase while one plays

{{shot: arp-phrase-next-bar | A phrase picked while another plays, with the transport running: Dub steps waits in amber and comes in on the next bar line; Night drive, in green, plays until then.}}

While the transport plays, a phrase you pick waits for the next bar line of the one playing and comes in there, so the change always lands in time. Until then its row is outlined in amber ("next bar"), the phrase still playing is marked in green ("playing"), and the arp chip in the top bar keeps naming the one you hear. Pick again before the bar line and the new pick waits in its place; pick the one that is playing and nothing changes at all.

With the transport stopped there are no bars to keep, so a pick comes in on the phrase's next step: turning the wheel through a list is quick to hear.

#### On a kit track

{{shot: arp-phrase-kit | The Phrase tab on a kit track: it says the track's instrument is a kit, where the list would be.}}

A kit's pads are different sounds, not notes, so there is nothing for a phrase to move: on a kit track the Phrase tab says so, and the pads play as pressed. NOTE REPEAT still repeats them, even if you left the panel on Phrase, and its **–** and **+** step Repeat, Arp and Pattern there.

A synth or kit you have chosen for the pads while the track plays something else plays as pressed too: see [When the pads play something of their own](#when-the-pads-play-something-of-their-own).

#### On the quick band

{{shot: arp-quick-band-phrase | Holding NOTE REPEAT on a synth track with the mode on Phrase: the band names the phrase, and its – and + step the speed (1× here).}}

Hold NOTE REPEAT on a synth track and press **+** until the band reads Phrase. The band names the phrase, its own **–** and **+** buttons step the speed (½×, 1×, 2×), and the data wheel moves through the phrases of the same kind. To change the kind, open the panel.

> [!TIP]
> Latch a bass phrase, then play single pads with one finger: the line follows you from note to note, and a new phrase picked on the panel drops in on the bar.

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
| **Modifier** | Opens the Modifier's settings: the controller that pushes the arp, and how far (the next section). |
| **Latch keeps pads**, **Sustain pedal latches** | The same rows as in Prefs: how Latch keeps pads, and whether the sustain pedal latches the arp (see [Latch](#latch)). |

## Push the arp with a controller

The **Modifier** lets one controller push the arp while you play: the mod wheel up and the arp runs faster and climbs an octave higher, back down and it plays as you set it. It is how a held chord builds into a rush, or a hi-hat roll gets tighter as you lean on the pads.

{{shot: arp-modifier | The Arp panel with the mod wheel 80 % up: the Modifier box beside the tabs shows it, and the sliders show what plays now beside what is set, 1/16 → 1/32, gate 50 → 74 %, 2 → 3 octaves. The step strip along the bottom climbs the extra octave.}}

1. Open the Arp panel and tap the **Modifier** box at the right of the tabs (or press **MENU** and pick **Modifier**).
2. Pick the **Controller**: **Mod wheel**, **Pressure** (how hard you press the pads, or the keys of a keyboard with aftertouch, after the hit), **Expression pedal**, **Breath**, or **Learned knob**. For a knob, hold **SHIFT**, touch the Modifier box, then turn the knob (see [MIDI Learn](midi-and-cv.md#midi-learn)).
3. Set how far the controller's top pushes each setting. Picking a controller already makes it 2 rates faster and an octave higher at the top.
4. Play, and move the controller.

{{shot: arp-modifier-options | The Modifier's settings: the controller and how far its top pushes the rate, octaves, gate and swing, and the rolls and accents it adds.}}

| Setting | At the controller's top |
|---|---|
| **Pushes rate** | Up to 4 rates faster: from 1/16, +2 is 1/32. Below zero, slower. |
| **Pushes octaves** | Up to 3 more octaves (4 in all). Below zero, fewer. |
| **Pushes gate** | Longer notes, up to 100 %. Below zero, shorter, choppier ones. |
| **Pushes swing** | More swing, up to 75 %. Below zero, straighter. |
| **Adds rolls** | Up to 7 extra hits in every step: a roll that builds as you push, great on drums with Note Repeat. |
| **Adds accents** | The chance that a step plays accented, at the Accent level. |

The push follows the controller smoothly: halfway up pushes about half as far. A new rate starts on the next step and stays in time with the beat. Your settings never change: the sliders keep showing them, with a ring where the arp plays now. The controller still reaches the sound as well, so the mod wheel can open a filter while it pushes the arp. Each track's arp has its own Modifier, saved with your project.

> [!TIP]
> On a drum kit: Note Repeat on the hi-hat, Controller **Pressure**, **Adds rolls** +3, and Pushes rate 0. Press harder into the pad and the hats roll.

The [XY pad](xy-layouts.md#the-arp-on-the-pad) can push the arp too: put the **Modifier** box on a layout of yours, and sliding your finger along pushes the arp by the Modifier's amounts, even with no controller picked. A layout can also hold the arp's rate, gate, swing or octaves where your finger puts them, and let go when you lift.

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
> On the MPC, Note Repeat and the arpeggiator share one window: hold **NOTE REPEAT**, tap **Arp**, and pick its Action (Arp, Note Repeat, Rhythm or Pattern). In MPFree NOTE REPEAT's – and + switch between repeat, arp and pattern (and on a synth track phrase) while you play, and the Arp panel takes the middle of the Play page while the transport bar, the master bar and the Q-Links stay on screen. Latch is a double press of NOTE REPEAT. The MPC's arpeggiator modifiers let several controllers push it at once; MPFree's Modifier takes one controller per track, can push the other way too (slower, fewer octaves, shorter notes), and adds rolls and accents. The MPC's Rhythm action plays the held chord in a rhythm from its list; MPFree's Chords and Gates groups do the same, and every one of its 74 patterns follows the notes you hold. You can also write your own step by step or tap them in, with ratchets, chance, ties and octave jumps.
