<!--
status: changing
covers: rust/mpfree-fx rust/mpfree-fx-ui rust/mpfree-chain-ui rust/mpfree-notefx rust/mpfree-app/src/chain_page.rs rust/mpfree-app/src/plugins.rs rust/mpfree-plughost/src/rack.rs rust/mpfree-mpchost rust/mpfree-fx/src/perform.rs rust/mpfree-fx/src/xyfx.rs rust/mpfree-fx/src/picker.rs rust/mpfree-chain-ui/src/picker.rs
checked: 2026-11-18 1a0b4bde
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
- **Up** and **Down** move the selected effect in the chain, and **Remove** takes it out. Its page shuts with it: the chain's list stays, with nothing picked, until you tap another effect.
- To move an effect further, **drag it** up or down the chain: once your finger moves it lifts and follows you, a line shows where it will land, and it moves there when you lift. Lift it where it started to leave it where it was. With the wheel, push on the picked effect to pick it up, turn to move it, and push again to put it down.
- The effect's page is in **sections**, each named and coloured for what it does: the effect's heart (its space, its echoes, its pitch) in cyan, tone and colour in coral, dynamics in lime, movement in pink, and the output in grey. Drag a knob, fader or slider to turn it; tap a choice's name, or step it with its arrows; tap a switch to flip it.
- A control that does nothing as the effect is set shows dim, and its section says why: a delay's **Time** with **Sync** on, for one.
- **Presets:** tap the effect's name at the top of its page for its presets: **Defaults**, then its own (for an effect with modes, the mode's). Tap one to load it; the effect's **Mix**, **Mix curve** and **Quality** stay as you set them. While the effect is set as a preset, the top of the page shows the preset's name, with the effect's beside it; once you change a control it shows the effect's name again.
- Most effects show all their controls on one page. The few with more have a tab for each page beside the effect's name; if there are more tabs than fit, **‹** and **›** step to the page before or after. The Channel strip's first page shows its stages (**Gate**, **Comp**, **EQ**) as sections, each with its switch: tap a stage's switch to turn it on or off, or **Open ›** for its page.
- The four controls with a **Q1** to **Q4** badge are the effect's **Q-Links**, the four it is played by. They stay the same on every page, so you keep the same controls under your hands while you look through the rest; when a setting takes a control out of play (a delay's **Time** when **Sync** is on), its Q-Link turns the control in its place. The data wheel works as Q-Link 1 until you tap a control: then it moves the outline, and a push on a knob lets the wheel turn it (see [The wheel's outline](../getting-started/the-screen.md#the-wheels-outline)).
- **‹** at the top left goes back to where you opened the editor: the Mix tab as you left it, or the Pads tab.
- Beside **‹**, the **strip button** names the strip whose chain this is (with its colour, and "Track 7" or "Pad 3" under the name). Tap it for the **strip list**: the strips by group (**Tracks**, **Pads**, **Inputs**, **Returns**, **Submixes**, **Master**), with **< >** to page through a long group. Tap a strip to open its chain instead, on its first effect, or with the list of effects open if it has none. **Back** closes the list and keeps the strip you were on.

The drawing of what the effect does sits in the section it belongs to, and changes as you turn: an EQ's curve, a delay's repeats, the Ducker's pump over two beats (or after a kick), a reverb's tail falling to silence over its Decay (the Room's early reflections spreading out as it grows, and cut short by its Gate; the Spring's echo coming back every Length), what a reverb's or the Tape delay's tone keeps (on the tape, each time round the loop, and dashed after four), the Tape delay's echoes from each head and round again, the Multi-tap's four taps on the beat, the Channel strip's curve (the gate under its threshold, then the compressor), its EQ and the order of its stages, the Transient shaper on a drum hit (dashed going in, solid coming out), where the Pitch shifter puts the sound, with the steps its feedback climbs, Pitch correct's steps onto the key's notes (with a dot on the note you are singing), how far each Harmonizer voice moves on each note of the key, and the Tuner's needle.

Adding, removing or reordering an effect while the music plays doesn't click: that strip dips out for a moment and comes back. If the chain is full or the mixer is busy, the hint line says so for a few seconds.

### The Modulate row

Touch a control on the page and the **Modulate** row under the page names it. Then:

- Tap a **macro number** (1 to 8) to have that macro move the control, and tap it again to take it off. The control shows the macro's badge (**M2**) at its corner.
- Tap **Learn**, then turn a Q-Link, or move a knob or fader on your keyboard, to give the control a control of its own (see [MIDI Learn](midi-and-cv.md#midi-learn)).
- Tap **+ LFO** or **+ Follower** to have an LFO or an envelope follower move it. A ring round the knob shows how far it moves. The row opens the control's **mod rows** over the page: drag a row's bar to set how far and which way it moves the control, and tap **x** to take it off. The line under the control's name says what moves it ("LFO 1 · +30 %"); tap it to open the rows again. A touch anywhere else closes them. A switch takes macros only.

### Choosing an effect

**+ FX** on a strip, or **+ Add effect** in the chain editor, opens the list of effects over the chain editor. The line at its top says where the effect will go: which strip, and which slot of its chain.

- Down the left are **Favourites**, **Recent** (the last 12 effects you added), then the groups (Dynamics, EQ and filter, Drive and colour, Modulation, Delay and reverb, Pitch and vocal, Performance, Plugins, and Utility), each with how many effects it has. Tap one to see its effects on the right. The list opens where you left it.
- **Tap an effect** to add it with its usual settings. On a send return (A to D), a delay or reverb starts fully wet, **Mix** at 100 %: the strips' own sound already reaches the master, so the return adds only the echo or reverb, and each strip's send sets how much you hear. On any other strip it starts at its usual Mix (25 or 30 %).
- **Hold an effect** (half a second: its presets open while your finger is still down), or tap its **presets** line, to see its presets: tap one to add the effect already set that way, or **Defaults** for its usual settings. **‹ Effects** goes back.
- **Tap the star** on an effect to add it to Favourites, and again to take it off. Favourites and Recent are kept when you turn the MPC off, for every project.
- **With the data wheel:** turn it to move through the effects, **SHIFT + turn** to move through the groups, and **push** it to open the effect's presets; push again to add the preset that is lit. The hint line reminds you.

**Cancel** closes the list without adding anything.

The Pads tab's **Effects** row opens the chain editor on the pad picked, in the Mix tab, and **‹** comes back to the Pads tab (see [Sampling](sampling.md#the-pads-tab-sounds-onto-pads)). The Sample tab's **FX** page shows the pad's effects as pedals, with their switches and first knobs, and its **Whole chain** opens the same chain editor on the pad you are editing, with a switch between the pad's own channel and the Drums track (see [Shaping how a pad plays](shaping-a-pad.md#fx-effects-on-one-pad-or-on-the-whole-kit)).

## MIDI effects: a track's Notes

A track that plays notes (not a Loop track) can change its notes before they reach its sound: make each note a chord, keep notes in a scale, move them, set how hard they play, make them all as long, echo them, loosen them up, let some through only by chance, strum a chord, hold notes after you let go, or move notes at random. These are its **MIDI effects**, and with the track's arp they are its **Notes**. They work on the pads, a keyboard and the track's clips, and on what the track sends out of MIDI.

{{shot: chain-notes | A track's chain: NOTES above SOUND, with Scale, Chord, the arp and Humanize (off) in order. Chord's page shows its shapes, what goes in and what comes out on a keyboard, and its settings; the Q-Links turn Chord, Voicing, Inversion and Velocity.}}

Open a track's chain in the chain editor (its strip's **+ FX**, or an effect in its insert row). The left column then has two groups:

- **NOTES**, the track's MIDI effects and its **Arp**, in the order the notes go through them, top to bottom. Each row has its name, a line on how it is set, and its **ON** switch. Four rows show at a time; **‹ ›** on the NOTES line scroll through more. The line says how many MIDI effects the track has out of 8.
- **Plays on**, between the groups, says where the track's notes go: **Its own sound**, or a MIDI out port and channel. Tap it to change that in the mixer's I/O view, in the MIDI section's **Tracks out** (see [Playing a synth from a track](midi-and-cv.md#playing-a-synth-from-a-track)).
- **SOUND**, the effects on the track's sound, as before.

**+ MIDI effect** opens the eleven MIDI effects as tiles, each with what it does. Tap one to add it, just above the arp, with its page open. **Cancel** closes the tiles.

| MIDI effect | What it does |
|---|---|
| **Scale** | Keeps notes in a scale: a note outside it moves down, up or to the nearest note in it, or isn't played |
| **Chord** | Adds notes to each one played: a shape (Octave, Fifth, Major, Minor, 7th, m7, maj7, sus2, sus4, add9, dim, aug, In key or your own), a voicing, an inversion, a highest note, the added notes' velocity and a strum |
| **Transpose** | Moves every note, up to four octaves either way |
| **Velocity** | Sets how hard each note plays: one velocity, or the one you played on a curve between a lowest and a highest |
| **Note length** | Makes every note as long, in ms or a note value, however long you hold it |
| **Echo** | Plays each note again after it, softer each time, up to 16 times, with a pitch step per repeat if you like |
| **Humanize** | Plays each note a little late and a little louder or softer, never the same way twice |
| **Chance** | Plays each note with a chance, from 0 to 100 % |
| **Strum** | Plays the notes of a chord one after another, 1 to 200 ms apart: **Up** (lowest first), **Down** or **As played**. Notes that start within 40 ms of each other are one chord. Up and Down sort a chord from a clip, the pads or a Chord effect above; keys you play by hand strum in the order you play them. Each note keeps its length |
| **Latch** | Keeps notes on after you let go. **Next chord**: they end when you next play with no key held, and the new notes stay instead. **Toggle**: press a note to turn it on, press it again to turn it off. Switch Latch off or remove it to end what it holds |
| **Randomize** | Moves some notes up or down at random (**Pitch**, up to 24 semitones, on **How often** of them), spreads how hard they play (**Velocity**) and makes them last a little longer (**Longer by**, up to 1 second), never the same way twice. A setting at 0 leaves that part alone |

Tap a MIDI effect's row for its page. Its choices are along the top as buttons (Chord's shapes, Scale's root), a keyboard shows what goes in and what comes out (for Chord, Scale and Transpose), and its settings sit in sections under it: tap a choice, or – and + to step a value, or rest a finger on a setting's row and turn the data wheel (see [The wheel's outline](../getting-started/the-screen.md#the-wheels-outline)). The four with a **Q1** to **Q4** badge are on the Q-Links. The page's line under its name says where it is among the Notes ("after Scale, before Arp").

- **Up** and **Down** move the row picked, the arp too. An effect above the arp changes the notes it gets; one below changes what it plays.
- **Remove** takes a MIDI effect out. The arp stays: tap its row for what it plays now and **Open the arp ›**, which opens the arp panel on that track.
- **ON** switches one off without losing how it is set.
- A key held while you change an effect ends as it started, so nothing hangs.
- The track's MIDI effects are saved with the project.
- On a drum track, the pads go through them but the track's clips don't.
- On a drum track the MIDI effects see pads, not notes: **Transpose** by 1 plays the next pad, a pad moved off the kit doesn't play, and **Randomize**'s Pitch plays other pads at random. Velocity, Note length, Echo, Humanize, Chance, Strum and Latch work the same on any track.

Touch a MIDI effect's setting and the **Modulate** row under the page works on it, as it does for a sound's knob: tap a macro number to put it on that macro, **Learn** to put it on the next Q-Link you turn, or **+ LFO** to sweep it (a choice, such as Chord's shape, takes a macro but not an LFO). A step can lock it, and while you record, a Q-Link turn of it records with the clip and plays back with it. What these do is heard from the next note; a note already playing keeps what it started as. Until you touch a setting, the row says which settings the Q-Links turn. Returns, submixes, the master and Loop tracks have no Notes.

## The effects

The list of effects groups them: Dynamics, EQ and filter, Drive and colour, Modulation, Delay and reverb, Pitch and vocal, Performance, and Utility (the Tuner, Stereo width and Sample delay). **Reverb**, **Delay**, **Drive**, **Filter** and **Tremolo** each come in several modes (see [Effects with modes](#effects-with-modes)).

| Effect | What it does |
|---|---|
| **Compressor** | Evens out levels and adds punch or glue. **Attack** is how long it takes to get most of the way down (63 %, the usual convention) and **Release** how long to come most of the way back. **Style** sets how it moves: **Clean** does exactly what the knobs say; **Opto** eases in and comes back slowly after long, heavy compression, for smooth levelling (vocals, bass); **Bus** comes back quickly after a hit and slower under a steady mix, for glue; **FET** adds a little grit as it turns down, more the harder it works, for drums and aggressive vocals. **Auto makeup** turns it back up by what it takes from a sound at a mix level, so comparing it with bypass is fair (**Makeup** still adds on top). **Range** limits how far it turns down; **Link** at 100 % moves both sides together, lower lets each side follow itself. It can be keyed from another strip, for side-chain pumping. **SC filter** keeps low end (a kick) from pushing it, so a whole mix doesn't pump; **Detect** picks **Peak**, which catches every hit, or **RMS**, which follows the loudness and is smoother on a bus or a vocal. |
| **Ducker** | The pump: turns the sound down on each kick (keyed from another strip) or on every 1/16, 1/8, 1/4, 1/2, bar, dotted or triplet beat of the tempo, and brings it back on a curve you shape. |
| **Transient shaper** | More or less punch at the start of each hit and more or less ring after it, at any level, with no threshold to set. |
| **Limiter** | A brick wall: nothing gets past the ceiling. **True peak** also catches the peaks a converter makes between samples; **Auto release** keeps a loud mix from pumping. See [The Limiter](#the-limiter). |
| **Gate** | Silences (or turns down) what falls under the threshold, so drums stop ringing; as an **Expander** it turns quiet sound down gently instead. It can open from another strip's sound, for rhythmic gating. |
| **De-esser** | Turns down harsh "s" and "t" sounds ([more](#the-de-esser)). **Listen** plays only what it hears, to find the right frequency. |
| **Multiband comp** | Three compressors, on the lows, mids and highs ([more](#the-multiband-comp)). |
| **Channel strip** | A gate, a compressor and an EQ in one, as on a mixing desk. Its first page, **Main**, shows the three as sections, each with its switch, then the **Signal path** (**Order**: the EQ before or after the gate and compressor) and **Output**; then a page each for the gate, the compressor and the EQ. |
| **EQ** | Shapes the tone with up to eight bands (each a bell, a shelf or a notch) and low and high cuts: cut mud, add air, take out a ring, thin out a sound. Its page is a graph with a dot for each band (see [The EQ's page](#the-eqs-page)); each band can be on (**ON**), switched off with its settings kept (**OFF**), or removed. Bands 1 to 4 start on and bands 5 to 8 start removed. |
| **Filter** | **Multi**: low-pass, high-pass, band-pass and more, with resonance. The resonance sings on quiet sounds and holds back on loud ones instead of clipping, as an analog filter does, so a full-Resonance sweep over a whole loop stays under full scale. Changing **Type** or **Slope** crossfades, so it doesn't click. **Drive** pushes the sound into a soft saturator before the filter, cleanly (no fizz on high notes). **Model** picks **Multi** or **Ladder**, a round, Moog-style 24 dB low-pass whose bass thins a little as the resonance rises and which, near the top of **Resonance**, sings on its own at the cutoff: play it like an oscillator by moving **Cutoff**. **Res mode** picks how the resonance holds back on loud sounds, on either model: **Soft** bends (the default, described above); **Hard** clips, for a harder, buzzier scream, as some classic analog synth filters do, while quiet sounds still ring up fully; **Clean** holds nothing back, for the deepest ring, but a loud sound at the cutoff can clip, and the ladder never sings on its own. Changing it while playing doesn't click. **DJ**: one knob, turn left for a low-pass, right for a high-pass. |
| **Comb** | Short tuned delays for metallic, resonant tones. |
| **Auto wah** | A wah that follows your playing: louder notes open it further. It can also sweep in time (**Source** LFO) or stay where you put it (**Source** Pedal: turn **Center** with a Q-Link, like rocking a pedal). It sounds about as loud as the dry signal at any **Resonance**, and you can change **Filter**, **Source** or **Resonance** while it plays without a click. |
| **Drive** | Warm saturation up to hard distortion: **Soft**, **Hard**, **Tube**, **Foldback**, **Fuzz** and **Rectify**; and the colour of a mixing desk or a tape machine: **Console**, **Tape**, **Iron** and **Soften**. Auto-gain keeps the level steady as you turn it up, so you hear more grit, not more volume. **Bias** makes the curve lopsided, for a fuller sound with even harmonics, in every mode but Soft (there it is dimmed). Changing the mode or the Quality fades over 10 ms, so it doesn't click while you play. |
| **Amp** | A guitar or bass amp (Clean, Crunch, Lead, Bass) with its tone controls and a speaker cabinet, from a clean sparkle to high-gain lead. **Gain** changes the grit, not the loudness: the amp stays about as loud as what goes in, and **Level** sets the rest. **Boost** puts a pedal in front: **Tight** (a mid-hump overdrive with the drive low, the low end cut, for tight high-gain chords) or **Treble** (a hot, thin treble booster). **Gate** shuts hiss in the rests (0 is off; higher closes on louder sounds), and **Release** is how fast it closes. **Sag** makes the power stage give a little under loud playing, as a tube amp's supply does; **Depth** lifts the low resonance. **Presence** brightens the top. Changing the model, the boost or the cabinet crossfades, so it doesn't click. |
| **Fuzz** | A fuzz pedal with three circuits: **Face** (two transistors, round and spitty), **Muff** (two clipping stages and a scooped tone, thick with long sustain) and **Octave** (an octave up on single notes, ring-mod clusters on chords). **Fuzz** changes the grit, not the loudness, and **Level** sets the rest. **Tone** darkens or brightens it; on Muff it moves from bassy to thin and bright, with the mids dipped in the middle. **Gate** starves the circuit, so quiet notes sputter and cut out (0 leaves it alone). **Low keep** keeps everything under 60 to 250 Hz clean, for 808s and bass, while the fuzz plays above it. Changing the model crossfades, so it doesn't click. |
| **Exciter** | Adds sparkle and air to the top end. |
| **Crush** | Bit and sample-rate reduction, from a touch of grit to a broken toy. **Bits** and **Rate** are the sound of old samplers and game consoles. **Tails**: **Cut** silences whatever falls under the smallest step, as most crushers do; **Buzz** keeps quiet tails as a buzz instead. **Filters** adds the low-passes an old sampler had before and after its converter (**Before**, **After** or **Both**), for a rounder, darker sound; the vintage presets have them on. **Jitter** wobbles the sampling clock. **Dither** turns the distortion of a few bits into a steady hiss. **Output** sets the crushed sound's level. |
| **Lo-fi** | Worn tape and dusty vinyl under one **Amount** knob, from a hint of age to a broken machine. **Wow** is slow pitch drift that wanders like a worn tape, not a steady wobble, at about **Rate**; **Stereo** lets the two sides drift apart; **Flutter** is a fast waver; **Saturation** is the tape's soft grit; **Dropouts** are short dips in level where the tape is worn. **Crackle**, **Noise** (hiss) and **Hum** (at your country's **Mains**, 50 or 60 Hz) follow the music while **Follow** is at 100 %: they come in with the sound and fade about a second after it stops, so a stopped track is silent. Turn Follow down for a bed of noise that keeps going. **Low cut**, **High cut** and **Width** set the tone. At Amount 0 it is exactly the sound going in (see [Lo-fi](#lo-fi)). |
| **Chorus**, **Flanger**, **Phaser** | Movement and width. Each is below. |
| **Tremolo** | Volume (**Volume**) or position (**Auto-pan**) moved in time with the music. **Harmonic** is the swirl of 1960s amps. Every **Shape** is smooth: a Square chops crisply at any speed without a click, and the ramps return in a few milliseconds. Synced, it stays on the beat when a loop starts again or you jump in the song, and changing Shape, Sync or Mode while it plays glides instead of clicking. |
| **Ring mod** | Bells, clangs and robot voices. **Sine** is the purest; **Triangle** and **Square** are buzzier but stay clean even with a high **Freq**. All three are about as loud, and you can change **Wave** while it plays without a click. |
| **Rotary** | A spinning organ speaker: switch **Speed** between Slow and Fast and it ramps up or down like the real one, the small horn first and the big drum after, slowing down more gently than it speeds up. **Ramp** is how long the drum takes. The horn's top end swirls the most, and it sounds as loud as the dry signal. |
| **Freq shifter** | Moves every frequency up or down by the same number of hertz, so a sound stops being in tune with itself: a few hertz is a slow, endless swirl, more makes drums metallic and voices robotic. **Feedback** sends it round again, climbing or falling, and dies away when the sound stops. Only the direction you pick is heard (the other is far below it), and changing **Mode** while it plays doesn't click. |
| **Delay** | Echoes in time with the tempo. **Digital** is clean; **Tape** echoes from a tape loop with up to three heads, wow and flutter: the first echo already sounds like tape, and each repeat darkens and thickens on from it; **Multi-tap** is four echoes on the beat, each with its own level and pan. |
| **Reverb** | **Room**, **Plate** and **Spring** are the smaller, classic reverbs (the room's **Gate** cuts its tail short, for the 1980s gated snare); **Hall**, **Cloud** and **Echoes** are large and lush. **Freeze** holds the tail for as long as it is on. |
| **Granulator** | Cuts the sound into tiny grains and plays them back as a cloud: higher or lower, scattered, backwards, spread wide. **Freeze** holds what's in it, so a moment hangs in the air while you play over it. |
| **Grains** | MPFree's grain engine as an effect, the same one the sampler's granular pads play: clouds of grains from the last eight seconds of what came in. **Position** sets how far back they read, **Size** and **Density** how long and how many, **Spray** how far they scatter; **Pitch**, **Pitch spray** and **Reverse** move them. **Feedback** sends the grains round again, so a trail builds and fades (it can't run away, even at 100 %), and **Freeze** holds what's there. **Repeat** is a stutter in time with the music: pick a length from 1/2 to 1/32 and, on the next beat line of that length, the last stretch of what came in plays again and again in place of the sound (at any Mix); pick a shorter length while it repeats and it rolls the start of that stretch, faster and faster; Off goes straight back. Pitch and Reverse play through it. **Capture ›**, in the Hold heading, puts what Grains holds on a pad (all eight seconds while frozen, else the last six): the pad a recording would go to (the pad picked if empty, else the bank's next empty one), named Grains 1, 2 and so on, playing as grains with the effect's settings, so you can play the cloud from the pads and keys. UNDO takes it off. The **Texture** page shapes each grain (**Shape**, from square through Hann to a swell or a pluck) and how much they vary: **Timing**, **Size spray**, **Level spray** and **Peaked spray** (most grains near the position). Its **Filter** section filters each grain on its own: **Filter** picks **Low pass**, **Band pass** or **High pass** (Off to leave them clean), **Cutoff** where it bites (20 Hz to 20 kHz), **Resonance** how much it rings there, and **Filter spray** gives each grain its own cutoff up to three octaves either side, so the cloud turns into many colours at once, from glassy to gnarly. The Texture page's Q-Links turn Timing, Shape, Cutoff and Resonance. Presets: Trail cloud, Octave shimmer, Reverse haze, Glitch dust, Octave down, Long smear, Glass spray (a sprayed high pass, an octave up) and Gnarl (a ringing low pass on short, rough grains). The older **Granulator** stays in the list, so projects that use it open as they were. |
| **Pitch shifter** | Moves the sound up to two octaves up or down without changing its speed. It joins the sound where it lines up, so every note comes out clean, steady and on pitch, and chords hold together. Window sets how far behind the dry it plays: about 11 ms at the default 30 ms, tight enough to layer an octave under a bass. Spread detunes left and right for a wide double; feedback makes rising or falling stairs. |
| **Pitch correct** | Tunes a voice to the notes of a key. Pick the **Key** and **Scale**; **Speed** sets how quickly it pulls a note in (0 snaps straight to it, for the hard, stepped sound; 50 ms or more keeps slides and vibrato natural) and **Amount** how far. **Flex-Tune** leaves notes you sing well away from the note alone, so slides and deliberate bends stay yours, and **Humanize** lets a long held note keep its vibrato while quick notes still snap in (both 0 by default, on the Voice page). The **Note** and **Correction** readouts show the note it hears and how far it is moving it. |
| **Harmonizer** | Sings along with you: up to two extra voices a 3rd, 4th, 5th, 6th or octave above or below each note you sing, that stay in the key (a 3rd is major or minor as the key has it), each with its own level and pan. **Glide** smooths a voice into its next note, and **Dry** is your own voice. The voices stay clean and in tune with you, without a buzz. |
| **Talk box** | Makes a sound say vowels, from "oo" to "ee". The vowel moves with how loud you play or in time with the music; **Formant** makes the mouth smaller or bigger. Best on rich sounds: synth chords, organs, guitars (see [The Talk box](#the-talk-box)). |
| **Perform FX** | The throws, stutters, beat repeats, tape stops and the XY pad, as an effect you can put on any strip. On the master it is what the Play page's throws play (see [Playing live](playing-live.md#throws)); on a pad, its 16 slots play stutters and repeats on that pad alone. |
| **XY FX** | One effect at a time played from two controls at once, X and Y: see [XY FX](#xy-fx). |
| **Tuner** | Tunes a guitar, a bass, a voice or a synth: put it on the strip the instrument comes in on (an input, usually) and play one note at a time. It shows the note, how many cents sharp or flat (the needle is green when you're within 3 cents) and the frequency. **Reference** sets the pitch of A, 440 Hz unless your band tunes to something else; **Mute** silences the strip while you tune. It doesn't change the sound and adds no delay. |
| **Stereo width** | Makes the sound narrower or wider, with the lows, mids and highs set apart: put the bass in the middle and open up the top. **Delay** widens a mono sound and still sounds right when the mix is played in mono. |
| **Sample delay** | Delays the left or the right side by a few samples, and flips either side's polarity: for lining up two microphones, or a quick stereo spread. |

### A delay's, reverb's or chorus's Mix

**Mix** sets how much of the echoes, the room or the chorus you hear against the sound going in: 0 is the sound alone and 100 % the effect alone. Every delay and reverb mode, and the Chorus, also has a **Mix curve** in its Output section, a small choice you step with its arrows, for how the two blend in between. Touch it and the hint line at the bottom of the screen says what the choice does.

- **Equal** (equal power, the default): the level stays the same as you turn Mix, so the effect comes in without the sound dipping half way.
- **Dry first**: up to half way, Mix only adds the effect, with the sound going in at full level; from half way the dry sound fades out.
- **Linear**: a plain crossfade, a little quieter in the middle.

Changing the curve glides from one blend to the other, so it doesn't click. A preset, or a switch to another mode, leaves the Mix curve as you set it.

> [!NOTE]
> Projects saved before the Mix curve open at **Equal**, so a delay or reverb set between 0 and 100 % sounds a little louder and wetter than it did (at Mix 35 %, about 3 dB). Pick **Linear** for the blend it had before. The Room reverb is also about 4 dB louder than before, as loud as the other reverbs.

### The chorus's, flanger's and phaser's sweep

All three move with an LFO, set in their **Movement** section:

- **Rate** sets how fast it sweeps, in Hz. With **Sync** on a note value it follows the tempo instead: 1/16 to 8 bars a cycle and, at the end of the list, the triplet and dotted eighths and quarters (**1/8T**, **1/8D**, **1/4T**, **1/4D**).
- **Shape** is **Sine** (the default: the sweep slows and lingers at each end) or **Triangle** (it moves at an even speed and turns round quickly at each end). On the chorus a Triangle bends the pitch up and down by a steady amount, the classic synth-chorus sound; on the flanger and phaser the notches glide at an even pace. Its corners are rounded, so the turn is quick but never a jump, and a change of Shape glides from one to the other.

Projects saved before Shape open on Sine, as they sounded.

### The Chorus

- **Delay** sets where the copies sit, from 2 to 40 ms; **Depth** sets how far they swing either way, up to 5 ms at 100 %. The two are separate: a longer Delay doesn't make the chorus wobble more.
- However fast you set **Rate**, the pitch never bends by more than about a semitone: at high rates Depth stops short of its full swing. Below that, deeper and faster wobble more, as on any chorus.
- **Voices** stacks up to four copies at different points of the sweep, and at full **Width** the two sides always differ, at every number of voices, so the sound spreads wide. **Width** 0 moves both sides together.
- The copies stay as bright as the sound going in, however they sweep; **Tone** takes the top off them when you want it warmer.
- Synced to the tempo, the sweep stays on the bar: when a loop jumps back, it glides back into place within a beat instead of drifting.

> [!NOTE]
> Choruses saved before this change sound a little different: at the same Mix they are about 3 dB louder (pick **Linear** for the old blend), with three or four voices they are wide, and Depth now means the same swing at every Delay, so a chorus with a long Delay wobbles less than it did and one with a short Delay a little more. The presets keep the swing they had.

### The Flanger

- **Delay** is the middle of the sweep (a pedal's Manual), from 0.1 to 10 ms, and **Depth** swings it up to two octaves either way, so the notches glide evenly up and down the sound. The sweep never goes past 20 ms, where a flanger turns into a doubler: from a Delay of 5 ms up, full Depth swings a little less, the same both ways.
- **Feedback** sends the copy round again: up for a metallic ring, down (below 0) for a hollow one. The flanger stays as loud as the sound going in at any Feedback and Mix: more Feedback sharpens the notches and the peaks between them, it doesn't make the sound louder.
- The notches stay deep all the way up, however the sweep moves, and the curve on its page is the one you hear.
- **Mix** is a plain crossfade: 50 % is the deepest notch.
- **Through zero** (off by default) is the tape flange, the sound of two tape machines playing together while one is slowed by a thumb on its reel. The sound waits 5 ms and the copy, flipped over, swings either side of it, so twice a cycle the two meet and, at Mix 50 %, the sound vanishes for a moment. **Delay** times **Depth** is how far the copy swings each way, up to the 5 ms wait. The mixer makes up the 5 ms, so the track stays in time with the others. Feedback rings at about 5 ms, so keep it low for the classic sound: the **Tape flange** preset has none. Turning it on or off glides, so it doesn't click.

> [!NOTE]
> Flangers saved before this change sound a little different: they go in at the level of the sound going in, so at the defaults they are about 2 dB louder than before and with Feedback near either end up to about 4 dB quieter; their highs are brighter; and with a Delay over 5 ms and a deep Depth the sweep stops at 20 ms.

### The Phaser

- **Center** is the middle of the sweep, from 100 Hz to 8 kHz, and **Depth** swings it up to two and a half octaves either way. **Stages** sets how many notches move through the sound: 2 stages make one, 4 make two (the classic pedal), and 12 make six.
- **Feedback** sends the sound round again: up for a sharper, vocal sweep, down (below 0) for a hollower one. The phaser stays about as loud as the sound going in at any Feedback: more Feedback sharpens the peaks between the notches, it doesn't make the sound louder. The lowest few hertz don't go round, so high Feedback never builds up rumble.
- **Width** moves the right side's sweep away from the left's, and **Stereo** (beside **Sync**, under the sweep's drawing) sets how far full Width goes: **Quarter** (a quarter of a cycle, the default) or **Opposite** (half a cycle, so one side's notches rise while the other's fall). Width 0 moves both sides together.
- **Mix** is a plain crossfade: 50 % is the deepest notch.

> [!NOTE]
> Phasers saved before this change sound a little different: at the defaults and in the presets they are 0.5 to 2 dB louder, and with Feedback near either end about 4 to 5 dB quieter. They keep their quarter-cycle stereo.

### A delay's repeats

- The repeats keep their brightness at any time and tempo. Only **Tone** (and on the Tape and Multi-tap, **Low cut**) takes the top and bottom off them, by as much as the page draws.
- **Saturation** roughens the repeats without making them louder, so **Feedback** is how much of each repeat the next one keeps: at 50 % each repeat is half as loud as the one before (or quieter, where Tone and Low cut take some away).
- On the **Tape**, every echo goes onto the tape, the first one too. So the first echo already has the tape's tone, head bump and saturation, and each repeat has been round once more than the one before. The Tone card draws one time round, filled, and four times round, dashed.
- Over 100 % Feedback the echoes grow until they meet a soft ceiling, and howl there until you bring Feedback down. The Tape's Feedback goes to 150 %, so it can run away fast (its Runaway preset is at 130 %); the Digital's goes to 110 %, a slow build.

> [!NOTE]
> Projects saved before this change sound a little different on the Tape delay: the first echo has the tape's tone now, and quiet repeats no longer grow louder at high Saturation, so the Dub echo preset dies away instead of ringing on.

### The Room

The **Room** reverb is a room you put the sound in: its first reflections off the walls, then a tail that thickens fast and dies away evenly.

- **Size** spreads the reflections apart and makes the room bigger; **Decay** is how long the tail takes to fall away (60 dB, to near silence) in the mids; **Damping** makes the highs die sooner, as soft furnishings do.
- **Early** sets the level of the first reflections. A sound on the left reflects first off the left wall, so it stays on the left in the room.
- **Diffusion**, a small control under Early and Gate, sets how quickly the reflections blur into a smooth wash: down low you hear separate early echoes, up high they melt together at once. 70 % is a natural room.
- **Mod** keeps the tail from ringing on a note with a very slow drift, too small to hear as a wobble.
- **Pre-delay** waits before the room answers. Turning it never bends the pitch: the room fades over to the new time.
- **Gate** cuts the tail that many milliseconds after the sound stops, for the 1980s gated snare.

> [!NOTE]
> The Room was rebuilt, so projects saved before it sound different on a Room: denser and smoother, with no slap at the start, and a source keeps its side. Every setting and preset still loads and does what it did; Diffusion starts at 70 %.

### The Plate

The **Plate** reverb is the studio plate: a big sheet of steel that rings with the sound. It has no walls, so no separate echoes: a hit turns into a dense, bright shimmer at once and dies away evenly.

- **Decay** is how long the tail takes to fall away (60 dB, to near silence) in the mids; **Damping** makes the highs die sooner; **Tone** darkens what goes in.
- **Size** makes the plate bigger: a deeper, smoother tail.
- A sound on the left starts on the left before it spreads, so it keeps its place.
- **Mod** keeps the tail from ringing on a note with a very slow drift, too small to hear as a wobble.
- **Pre-delay** waits up to 250 ms before the plate answers, the classic vocal trick. Turning it never bends the pitch: the plate fades over to the new time.

> [!NOTE]
> The Plate was rebuilt, so projects saved before it sound different on a Plate: dense from the first moment instead of grainy, smoother in the tail, and a sound keeps its side. Every setting and preset still loads and does what it did.

### The Spring

The **Spring** reverb is the tank in a guitar amp or an old organ: the sound sent down a coiled spring and back, again and again. Each time round it chirps (the "boing", the lows arriving after the highs), which is what makes it lovely on guitars, organs and dub, and twangy on drums.

- **Length** is how long the spring is: an echo comes back every Length.
- **Decay** is how long the tail takes to fall away (60 dB, to near silence) in the mids; the lows ring on a little longer, as a real spring's do.
- **Tension** sets how much each echo chirps.
- **Drip** sends some of each chirp round the spring again at once, for the splash a tap on the tank gives (dub's favourite). It doesn't change how long the tail rings.
- **Tone** darkens what you hear; **Low cut** keeps the bass out of the tank.
- A sound on the left plays more on the left spring, so it keeps its place.

> [!NOTE]
> The Spring was rebuilt, so projects saved before it sound a little different on a Spring: Decay is what it says, an echo comes back every Length, and a sound keeps its side. Every setting and preset still loads and does what it did.

### Hall, Cloud and Echoes

The **Hall**, **Cloud** and **Echoes** reverbs are the large ones. The **Hall** is a concert hall: a spread of early reflections, then a smooth tail that builds slowly. The **Cloud** is an endless ambient wash for pads and long fades. **Echoes** is a few distinct repeats that blur as they fade, somewhere between a reverb and a delay.

- **Decay** is how long the tail takes to fall away (60 dB, to near silence) in the mids, and low notes no longer ring on after the rest. **Damping** makes the highs die sooner.
- Turning **Decay** changes how long the tail rings, not how loud it is, so you can set the length without riding the level.
- **Size** makes the space bigger; **Pre-delay** waits before it answers. Turning either never bends the pitch: the reverb fades over to the new size or time.
- **Diffusion** sets how quickly the echoes blur into a wash; **Modulation** and **Mod rate** keep the tail from ringing on a note with a slow drift.
- **Early** sets the level of the first reflections; **Width** goes from mono to wide; **Low cut** and **High cut** shape what goes in.
- A sound on the left stays on the left as the reverb starts.

> [!NOTE]
> Projects saved before this change sound a little different on a Hall, Cloud or Echoes (and on a Cloud reverb set to its own Plate or Room): Decay is what it says at every setting, a long Decay is no louder than a short one, and the Hall's tail is smoother at big sizes. Every setting and preset still loads and does what it did.

### The EQ's page

{{shot: chain-mixer-drums-eq | The EQ on the drum bus: its curve with a dot for each band and the cuts, band 2 picked in the panel underneath.}}

The EQ's page in the chain editor is a graph of what the EQ does, from 20 Hz on the left to 20 kHz on the right and from -18 to +18 dB, with a dot for each band (numbered 1 to 8) and for the low and high cuts (**LC**, **HC**). The white line is what you hear; each band's shape is shaded in its colour.

- **Drag a dot** to move its band: sideways for its frequency, up and down for its gain. While you hold it, a bubble shows its values.
- **Turn the data wheel** on a dot, or put a **second finger** on the screen while you hold one, to make the band narrower or wider (its Q). On a cut, the wheel switches between 12 and 24 dB.
- **Tap the graph twice** where there is no dot to add a band there.
- Under the graph, tap a band's chip to pick it, **+ Band** to add one, and a type to change it: **Low cut**, **Low shelf**, **Bell**, **Notch**, **High shelf** or **High cut**. **ON** switches the band off and on again (it keeps its settings), **Remove** takes it away, and **Solo band** lets you hear only the part of the sound that band shapes, to help you find a ring or a boom. Solo band follows the band you pick, and stops when you tap it again or leave the page.
- The Q-Links turn the picked band's frequency, gain and Q, and the EQ's output.
- **Quality**, at the end of the row of types, is for the whole EQ. **Normal** is right for almost everything. **HQ 2x** works the bands at twice the rate, so a broad bell or shelf up high follows the analog curve exactly, right up to 20 kHz (at Normal it can be up to about 1 dB off above 10 kHz). It takes about 1.7 times the EQ's share of the MPC's power, so keep it for the EQs where the top end matters, such as on the whole mix. A tap swaps the two without a click; the low and high cuts work the same either way.

How the bands are measured, the same as on most EQs, so settings you know carry over:

- A bell's **Q** is its width between the points where it has half its gain in dB: Q 1.41 is one octave wide, Q 0.71 about two, Q 4 about a third of an octave.
- A shelf's frequency is where it is at half its gain: a +6 dB high shelf at 8 kHz is +3 dB at 8 kHz.
- The bells and shelves keep their shape all the way up, like an analog EQ's: a broad bell at 12 kHz is as wide as one at 1 kHz, and still lifts the air above it, where many digital EQs squeeze it narrow near the top.

The Mix tab's **EQ** look shows every strip's EQ at once (see [Mixing](mixing-and-effects.md#every-eq-at-once-eq)).

### Pitch correct and the Harmonizer

Pitch correct and the Harmonizer listen for one voice: a singer, or a solo instrument such as a whistle or a sax, not chords. Set **Range** to fit it: **Low** for bass and baritone voices, **Mid** for most singing, **High** for high voices.

They hold the sound back a little while they listen (about 40 ms on Low, 28 on Mid and 17 on High), and the mixer holds everything else back to match, so the mix stays in time. When you sing live through one, High has the shortest delay. Changing Range while it plays dips the sound for a moment, since it starts listening again.

The **Natural** preset is a gentle pull for singing: notes you mean land in tune, while slides and the vibrato on long notes stay as you sang them. **Hard tune** snaps every note, for the stepped, robotic sound.

> [!NOTE]
> Projects saved before this change sound a little different through Pitch correct and the Harmonizer: a note held in tune comes out exactly as it went in, a corrected note is smoother and steadier, and the harmony voices are cleaner. Speed now takes as long as it says, so a saved Speed pulls in a little more slowly. Flex-Tune and Humanize start at 0, so a saved Pitch correct corrects as it did.

### The Talk box

The Talk box shapes a sound into a vowel, the way your mouth does when you sing "ah" or "ee". Pick the **Vowel** it rests on; **Env depth** moves it along the row (OO, OU, AW, AH, UH, AE, EH, IH, EE) as you play louder, and **LFO depth** moves it in time or at its own rate, so it talks. **Formant** makes the mouth smaller (up) or bigger (down), and **Resonance** makes the vowels sharper and more nasal or softer. Every vowel sounds about as loud as the sound going in, so sweeping through them doesn't pump, and turning any control while it plays glides instead of clicking.

It works best on sounds with plenty of harmonics: a saw lead, a synth chord, an organ, a distorted guitar. A pure sine has little for the vowels to shape.

> [!NOTE]
> Projects saved before this change sound different through the Talk box: it goes in at the level of the sound going in, about 6 dB louder than it was (5 to 8 dB in the presets), and the vowels are fuller and clearer, with no hollow dip between their formants. A Formant shift now makes the vowels a little wider or narrower along with the mouth.

### Lo-fi

Lo-fi plays about as loud as the sound going in, so switching it on and off compares the character, not the level. Its tape needs a moment of sound to bend, so it holds the sound back 10 ms, and the mixer holds everything else back to match, so the mix stays in time.

> [!NOTE]
> Projects saved before this change sound a little different through Lo-fi: high notes no longer fizz through the saturation, the wow wanders instead of wobbling evenly, and it is 1 to 1.5 dB louder than before (as loud as the sound going in). Stereo, Dropouts and Hum start at 0 and Follow at 100 %, so nothing new is added to a saved Lo-fi.

### The Limiter

**Gain** pushes the sound into the Limiter and **Ceiling** is as loud as it can come out. **Release** is how long the level takes to come most of the way back (63 %) after a peak; before it lets go, it holds for a moment (a quarter of Release, at most 5 ms), so a bass note isn't turned up and down inside each wave, which would make it buzz. With **Auto release** on (the default), it comes back in about a third of Release after a single hit and takes about three times Release after a long loud stretch, so a pushed mix doesn't pump. With **True peak** on, it also holds the peaks a converter rebuilds between samples at the ceiling, which is what a streaming service or a mastering engineer measures.

It holds the sound back about 1.8 ms to see the peaks coming, and the mixer holds everything else back to match, so the mix stays in time.

> [!NOTE]
> Projects saved before this change sound a little different through the Limiter: pushed hard, it is cleaner on bass and a little quieter (about 0.6 dB on a mix pushed 6 dB), and Auto release now acts.

### The De-esser

**Freq** is where the "s" sounds start; the De-esser listens above it, on whichever side is louder, so a doubled vocal panned to one side is caught as well as one in the centre. **Threshold** is how far that band has to stand out: with **Detect** on **Relative** (the default) it is measured against the whole sound, so it works the same on a quiet take and a loud one; **Absolute** uses the band's own level. **Range** is the most it turns down. **Split** turns down only what is above Freq and never touches the voice under it; **Wide** turns the whole sound down, which some voices like better. **Listen** plays only the band it hears, to find the right Freq.

It holds the sound back 2 ms so it is already turning down when each "s" arrives; the mixer holds everything else back to match.

> [!NOTE]
> Projects saved before this change sound a little different through the De-esser: in Split it no longer lifts the voice under Freq, it turns the "s" down as far as Range says (it used to fall well short), it catches each "s" from its start, and an "s" on one side is turned down as much as one in the centre.

### The Multiband comp

**Low | mid** and **Mid | high** split the sound into three bands, and each band has its own **Threshold**, **Ratio** and **Gain**; **Attack** and **Release** are shared and mean the same as on the Compressor (most of the way, 63 %, in the time shown). **Solo** plays one band alone while you set it up. **Mix** blends in the sound before compression for parallel squash, and any Mix stays even across the bands (the blend no longer dips at the split points). **Auto makeup** turns each band back up by what it takes from a sound at a mix level, so comparing it with bypass is fair; it is off at first and on in every preset, and **Gain** still adds on top.

> [!NOTE]
> Projects saved before this change sound a little different through the Multiband comp: it compresses as far as its curve says (it used to fall short, and react about 1.7 times slower than Attack showed), a Mix under 100 % no longer dips around the split points, and turning a threshold or ratio is smooth. The Drum crush preset now uses Auto makeup instead of +6 dB on each band.

### The Gate

**Threshold** is the level that opens it. It shuts again 4 dB under, so a sound hovering at the threshold doesn't flutter, once the sound has stayed under that for **Hold**. Hold is at least 20 ms even at 0, so a bass note never shuts it between the peaks of its waves. **Attack** is how fast it opens; it looks 1 ms ahead, so the front of a hit is kept. **Release** is how long it takes to fall to **Range**, the most it turns down (at the bottom, silence). Moving Range while the gate is shut takes effect at once.

**Mode** picks how it turns down. **Gate** shuts all the way to Range. **Expander** turns the sound down gradually under the threshold, by **Ratio**: at 2, a sound 3 dB under the threshold comes out 6 dB under, at most down to Range. It is a gentler clean-up for guitars, vocals and room noise; the Soft cleanup preset uses it.

**Key HP** and **Key LP** filter what it listens to, so a kick's thump or a hi-hat's hiss doesn't open it, and **Key** picks the strip's own sound or another strip's (**Sidechain**), for rhythmic gating. **Listen** plays what it listens to, so you can set the filters by ear.

> [!NOTE]
> Projects saved before this change sound a little different through the Gate: with Hold under 20 ms it now stays open through low notes (it used to flutter on bass under 100 Hz), it shuts closer to 4 dB under the threshold, and a Range moved while it is shut takes effect. Saved gates stay gates.

### The Channel strip

The **Gate**, **Comp** and **EQ** switches fade their stage in or out over 20 ms, so switching one mid-note doesn't click; the gate comes back on open and shuts from there if the sound is quiet. Changing **Order** dips the sound for a moment (about 20 ms) and swaps the stages in the gap. The compressor has the Compressor's **Style** (Clean, Opto, Bus, FET) and **Auto makeup**, which turns it back up by what it takes from a sound at a mix level, so comparing with bypass is fair; Auto makeup is off at first and on in every preset. Its gate holds open through low notes as the Gate does.

> [!NOTE]
> Projects saved before this change sound a little different through the Channel strip: its gate stays open through bass notes and shuts closer to 4 dB under its threshold. They keep the Clean style and their own Makeup. The presets now use Auto makeup instead of their own makeup, and a style each: Vocal and Bass Opto, Snare FET, Gentle glue Bus, Kick Clean.

### The Ducker

**Source** picks what makes it duck. On **Sidechain** it listens to another strip (usually the kick) and ducks when that goes over **Threshold**; **Key HP** and **Key LP** filter what it listens to, so with Key LP around 120 Hz it ducks on the kick of a whole drum bus and not on the hats (the Kick from drums preset). On **Tempo** it ducks on every step of **Rate**, from 1/16 to a bar, or a dotted or triplet eighth or quarter, in time with the song, and it is already down when each beat lands: it starts going down **Attack** before the beat. **Nudge** moves the duck up to 20 ms later or earlier, for a kick that sits behind or ahead of the beat.

**Depth** is how far it turns down, **Hold** how long it stays there, and **Release** how long it takes to come back, on a curve **Shape** bends from straight to a slow start and fast finish, the classic pump. If the Release is longer than the step, the duck never comes all the way back before the next one, and the drawing shows it as you hear it.

> [!NOTE]
> Projects saved before this change sound a little different through a Ducker on Tempo: each duck now starts Attack earlier, so it is fully down on the beat (2 ms earlier at the default Attack), and one with a Release longer than its step pumps as its drawing shows.

### The Transient shaper

It has no threshold: it follows the shape of the sound, so the same setting does the same to a quiet hit and a loud one. **Attack** turns the start of each hit up (more punch, more click) or down (softer, further away); **Sustain** turns up or down what rings on after it (more room and body, or a tighter, drier hit). **Speed** is how long the start lasts: short for a snappy click, long to take in the whole thump of a kick. **Output** sets the level after it and **Mix** blends it with the dry sound. A steady sound, a held pad or bass note, passes untouched.

At the default (Attack 50) every hit's front comes up by the same amount, about half of what Attack 100 gives; turn Attack to 0 for no change.

> [!NOTE]
> Projects saved before this change sound a little different through a Transient shaper: quiet hits now get as much lift (or softening) as loud ones. At Attack 50 a loud hit's front comes up about 7 dB less than it did, and a quiet one's a little more.

### Master safety

The master strip always ends in **Master safety**, which keeps the output from going past its **Ceiling** (-0.3 dB), including the peaks a converter rebuilds between samples. It isn't in the list: it is always there. Its **Mode**:

- **Safe limiter** (the default): a limiter that is down before a peak arrives.
- **Soft clip**: rounds the loudest moments off from 6 dB under the ceiling, a warmer, denser sound, and still never goes past the ceiling.
- **Raw**: no limiting. Anything over 0 dB clips harshly in the converter. Only for when you want that.

Changing the mode crossfades, so it doesn't click. **Trim** turns the whole mix up or down going in, and **Release** is how long the limiter takes to come back. See [The master and Master safety](mixing-and-effects.md#the-master-and-master-safety).

> [!NOTE]
> Projects saved before this change sound a little different when the master is pushed into Master safety: a little quieter and cleaner, with no peaks between samples over the ceiling. A Soft clip master no longer makes a gritty fold-back on loud high notes.

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

- **Automation lanes**: knob moves recorded as smooth lanes. Today a move made while recording goes into the clip as a lock on its step (see [Knob moves while you record](clips.md#knob-moves-while-you-record)).
- **Perform FX from the pads**: effects played by hitting pads.

> [!MPC]
> MPFree's effects open in the chain editor over the mixer rather than in a separate insert window. Effects with a delay of their own, such as Pitch correct, are lined up with the rest of the mix automatically, so nothing drifts out of time.
>
> Every MPC effect has an MPFree effect or preset in its place: the MPC's many Delay versions (Mono, Sync, Ping Pong, Analog, LP, HP) are the **Delay** and its presets, Delay Tape is the **Delay** in its **Tape** mode, Reverb Small, Medium and In Gate are the **Reverb** in its **Room** mode, Reverb Large and AIR Reverb its **Hall**, AIR Spring Reverb its **Spring**, Distortion Amp the **Amp**, AIR Enhancer the **Exciter**, AIR Maximizer and AIR Limiter the **Limiter**, AIR Noise Gate the **Gate**, AIR Pumper and Mother Ducker the **Ducker**, Decimator and Resampler **Crush**, the AIR Vocal Tuner **Pitch correct**, the AIR Vocal Harmonizer the **Harmonizer**, XYFX is **XY FX**, Auto Wah the **Auto wah**, the Frequency Shifter and AIR Freq Shift the **Freq shifter**, AIR Stereo Width the **Stereo width**, AIR Talkbox the **Talk box**, AIR Channel Strip the **Channel strip**, Granulator the **Granulator**, and Sample Delay the **Sample delay**.
