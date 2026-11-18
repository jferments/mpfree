<!--
status: changing
covers: rust/mpfree-sampler/src/model.rs rust/mpfree-sampler-ui/src/play_tab.rs rust/mpfree-sampler-ui/src/grain_tab.rs rust/mpfree-sampler-ui/src/keys_widgets.rs rust/mpfree-grain rust/mpfree-app/src/sample_page.rs
checked: 2026-11-16 ececc7e
-->

# Shaping how a pad plays

Once a sound is on a pad (see [Sampling](sampling.md)), the Sample tab's **Play**, **Grain** and **FX** pages decide what happens when you hit it: how loud and how bright, which other pads it stops or brings in, whether it plays as grains, and which effects it goes through. The last part of this chapter turns a whole kit into a keyboard instrument.

As everywhere on the Sample tab, **hit a pad** to pick the one you are shaping.

## Play: layers, envelope and filter

{{shot: sample-play | The Play page on its Amp section: the volume envelope drawn across the page over the sound, and under it the Level and Envelope knobs beside the scenes.}}

The **Play** page shapes how the picked pad sounds each time you hit it:

- **Layers:** different sounds for soft and hard hits. Drag a layer's edges to set its velocity range.
- **Envelope:** how the volume rises and falls: **attack**, **hold**, **decay**, **sustain** and **release**. Drag the points on the drawing, or use the Q-Links.
- **Filter:** darken or thin out the sound. Drag the dot.
- **Movement:** an LFO that wobbles the pitch, filter, level or pan, and a pitch envelope that drops or rises at each hit.

Its settings come in **sections**, a button each across the top of the page, with what is in each one written under its name: **Layers**, **Amp**, **Filter**, **Pitch**, **Mod**, **Movement** and **Pad**, and **Slices** for a sliced pad (see [Sampling](sampling.md)). Tap one and the page shows that section:

- **Its picture, across the page:** the layer map on Layers, Mod and Pad; the volume envelope over the sound on Amp and Pitch; the filter beside its envelope on Filter; the LFO's wave beside the pitch envelope on Movement; and the keyboard on Pitch and Pad while the pad is a keygroup or the pads play notes.
- **Its controls, in the panel under the picture:** knobs (drag up or down), buttons for a choice, ‹ › selectors, − + steppers and switches, in labelled groups as on a hardware sampler. A finger on a control gives it the wheel and **–** / **+**. The scenes sit beside them on every section.
- **Q-Links 1 to 4** turn that section's four main settings. The hint line names them.

| Section | Its controls | Q-Links 1 to 4 |
| --- | --- | --- |
| **Layers** | how a hit picks layers, Vel fade, the picked layer's Sound, Vel low, Vel high, Level, Pan and Tune, and Add layer and Remove layer | the layer's level, pan, tune and lowest velocity |
| **Amp** | the pad's Level and Pan, Attack, Hold, Decay, Sustain and Release | attack, decay, sustain, release |
| **Filter** | the filter's type, Cutoff, Resonance, Drive (the Ladder's), and its envelope: Env depth, Env attack, Env decay, Env sustain, Env release | cutoff, resonance, env depth, env decay |
| **Pitch** | Tune, Wow & flutter, Vintage, and the keys: Keys, Low key, High key, Root key, Key fade, and Mono, Legato, Glide and Glide on | tune, wow, low key, high key |
| **Mod** | Vel → level, Vel → start, Vel → cutoff, Vel → pitch, Pressure and its Depth | vel → level, vel → start, pressure, depth |
| **Movement** | LFO to, LFO wave, LFO rate, LFO depth, Sync to tempo, Retrigger, Pitch env and Pitch decay | LFO rate, LFO depth, pitch env, pitch decay |
| **Pad** | Polyphony, Mute group, Stops, Also plays, Latch, and the kit's Pads play, Pads from and Levels by | voices, mute group, pads play, pads from |

On a Q-Link, SHIFT turns finer. A setting that has nothing to change right now (the filter's while it is off, the keys' while the pad isn't a keygroup) shows its control and its card faint, and a tap on it says why.

On the **Filter** section the drawing shows the filter's own envelope instead of the volume's: how far and how fast the cutoff moves on each hit. **Env depth** says how far, up to 4 octaves up or down; drag the points for its times, or turn their knobs.

A cutoff or resonance change reaches a sound that is still playing, gliding there, and so does Drive; a new envelope or filter type plays from the next hit.

### Filter types: low pass to vowel

The **Filter** selector steps through the types:

- **LP**, **HP** and **BP** (low pass, high pass, band pass), each **2-pole** (gentle, 12 dB an octave) or **4-pole** (steep, 24 dB an octave). Low pass darkens, high pass thins, band pass keeps a slice around the cutoff.
- **Notch** (2-pole or 4-pole): the opposite of band pass. It takes out a band around the cutoff and leaves the rest, for a hollow, phasey sound; sweep it with the envelope or the LFO. 4-pole takes out a wider band.
- **Ladder**: a 4-pole low pass in the style of the classic analog synth filter, warm and round. Its Resonance sings: turned right up, it rings at the cutoff on its own, at a steady level, however long you hold the note. **Drive** pushes the sound into the filter harder, from Off up to 24 dB, for grit and growl. It keeps the level of an ordinary sound about the same as you turn it, so turning it up adds bite rather than volume, and it stays clean of the harsh, out-of-tune fizz cheap drives add on high notes. Drive is the Ladder's alone: on another type its knob is faint and says so.
- **Vowel**: two narrow peaks where a voice has them, so the sound "says" a vowel. Cutoff picks the vowel: from 200 Hz and below it says "oo", then "oh", "ah" and "eh", up to "ee" from 5 kHz, sliding smoothly between them, and the Cutoff's value names the vowel ("1.00 kHz · ah"). Resonance makes the peaks narrower and more nasal. Sweep the Cutoff with the filter envelope or the LFO and a pad talks: "oo-ah" on each hit. It works best on rich sounds (pads, saws, choirs, breaks); a pure tone right on a peak comes out louder.

{{shot: sample-play-ladder | The Filter section with the Ladder: a 1 kHz cutoff, Resonance at 70 %, so the curve peaks at the cutoff, and 12 dB of Drive.}}

> [!MPC]
> The MPC offers many more filter models. MPFree keeps a smaller set and measures each one in its quality lab against the analog filter it follows.

### Layers: more than one sound on a pad

{{shot: sample-play-layers | The Layers section: a second layer added for the hard hits, picked, turned down 3 dB and panned left; a hit at 100 lands on it.}}

A pad can hold up to 16 sounds, its **layers**: soft and hard hits of a snare, a kick with a click on top, round robins that take turns.

- **Add layer**, in the Layers section's panel, adds a copy of the picked layer just after it. The copy takes the upper half of its velocities, so a soft hit plays the first and a hard hit the new one. Change its sound with the **Sound** selector: it steps through the kit's sounds, and the new sound plays from start to end, keeping the layer's play mode.
- **Tap a layer** on the map to pick it. The Layers section's controls then set that layer alone: **Sound**, **Vel low** and **Vel high** (the velocities it plays; dragging its edges on the map does the same), and its own **Level**, **Pan** and **Tune**, on top of the pad's. The **Wave**, **Warp**, **Chop** and **Grain** pages then work on that layer too: its trim, loop, warp, slices and grains.
- **Remove layer** takes the picked layer out. A pad keeps one layer at least.
- The **Layers** buttons say how the layers that match a hit play: all together (**Stacked**), in turn (**Cycle**), or one at random (**Random**).

Each add, remove and change is one step for **UNDO**.

### Pads that stop, play or hold others

{{shot: sample-play-links | The Play page's Pad section with Also plays picked: hits on A1 and A3 put them on it, so a hit on A5 plays all three, and A5 latches.}}

Three controls of the Play page's **Pad** section set how a pad works with the others, and with your hands:

- **Stops:** the pads this one cuts off when you hit it. It works one way: the closed hi-hat stops the open one, and the open one leaves the closed one alone. (A **mute group** cuts both ways.)
- **Also plays:** the pads that play whenever you hit this one, at the same strength, as the MPC's Simultaneous Play does: a kick with its own sub, a stab with a noise burst, a chord from three pads. Each plays as a hit on it would, with its own layers, stops and latch, and lets go when you let go of this pad. The pads it plays don't play their own Also plays pads in turn. While the pads play notes or 16 Levels, Also plays does nothing.
- **Latch:** at **On**, a hit starts the pad and it plays on after you let go; the next hit stops it. A loop keeps looping and plays its tail when it stops, a grain cloud keeps going, a long pad or a drone holds, and your hands are free for the rest. A latched pad stays lit on the MPC.

To fill in **Stops** or **Also plays**, tap its selector, then hit the pads you want: each hit adds a pad, and a second hit takes it out again. Every hit is one step for **UNDO**. Tap another control when you are done, and hits pick the pad to edit again. The arrows and **–** and **+** work too (and the wheel, once you push it): a step up adds the next pad, a step down takes the last one out.

> [!TIP]
> Note Repeat plays a pad again on every step, so a latched pad would turn on and off with each one. Keep Latch for pads you hold, and Note Repeat for pads you roll.

**FULL LEVEL** and Half Level play every hit at full or at half strength, sampled pads included (see [Buttons and controls](../reference/controls.md)).

### Pitched up: clean or Vintage

Play a sound higher than it was recorded (**Tune** up, a key above a keygroup's root, pressure on pitch) and its top end moves up past what the MPC can play. On old samplers that part folds back down as harsh, metallic tones that were never in the sound (aliasing): a hi-hat pitched up an octave gains a whine. MPFree filters it out as it plays, so pitched-up hats, breaks and keys stay clean. A sound recorded at a higher rate, such as a 96 kHz file, plays clean too, and so do the grains of a granular pad whose **Pitch** or **Pitch spray** takes them up.

The **Vintage** switch of the Pitch section turns that off for one pad, its grains included. At **On** the pad plays pitched up the way old samplers did, grit and all, for when that is the sound you want. Vintage is saved with the kit.

Up to two semitones up, clean and Vintage sound exactly the same: what folds back there is above hearing. Past that, a clean pad takes a little more of the MPC's power for each sound it plays, and a Vintage one doesn't.

### Movement: an LFO and a pitch envelope

{{shot: sample-play-movement | The Movement section: a sine LFO on the pitch at 5 Hz, a vibrato, and a pitch envelope that starts an octave up and drops back over 200 ms.}}

The **Movement** section makes a pad move while it plays, the way the LFO and the pitch envelope of a hardware sampler do. Its picture draws two cycles of the LFO's wave beside the pitch envelope, and says what each does. Both start off.

**The LFO** moves one thing over and over:

- **LFO to** picks what: **Pitch** (a vibrato, up to 12 semitones either way), **Cutoff** (a filter wobble, up to 4 octaves either way, on a pad with a filter), **Level** (a tremolo, down to silence at full depth) or **Pan** (from side to side). **Off** stops it.
- **LFO wave** is the same set as the Grain page's: **Sine**, **Triangle**, **Saw up**, **Saw down**, **Square**, **Hold** (a new random value each cycle, held) and **Drift** (a new random value each cycle, glided to).
- **LFO rate** runs from 0.05 to 40 Hz. With **Sync to tempo** on, it is a note length instead, from 4 bars to 1/32, with dotted and triplet values, and follows the project tempo.
- **LFO depth** says how far, up to 100 % either way: a minus depth moves the other way. At 10 % on Pitch the LFO is a gentle vibrato of just over a semitone.
- **Retrigger**, at **On**, starts the wave from the top at each hit, so every hit wobbles alike. At **Off**, every hit joins one wave that runs on between hits, and a synced one keeps to the beat while the transport plays: a filter that opens on every bar, whichever hit lands where.

**The pitch envelope** bends each hit: **Pitch env** sets where it starts, up to 24 semitones up or down, and **Pitch decay** how long it takes to come back to the pad's pitch, from 1 ms to 10 s. A few semitones down over 50 ms gives a kick its punch; an octave up over a second makes a riser out of any sound.

Velocity can move the pitch and the filter too, on the **Mod** section: **Vel → pitch** sets how many semitones lower the softest hit plays than the hardest (a minus value plays soft hits higher), and **Vel → cutoff** how much darker a soft hit is, up to 4 octaves at 100 %.

A pad that keeps its pitch (warped to the tempo, or played as grains) doesn't follow the pitch envelope, Vel → pitch or an LFO on Pitch, and those controls show faint. A change to the LFO reaches a sound that is still playing; the pitch envelope and the velocity settings play from the next hit. Scenes morph the LFO depth and the pitch envelope with the rest.

### Scenes: mangle, morph and go back

{{shot: sample-play-scenes | The scenes on the Play page: A and B stored, the crossfader 40 % of the way to B.}}

The **Scenes** group, on the right of the **Play** page's panel on every section, holds two **scenes** of the whole kit, as on the Octatrack:

- **Store A** keeps every pad of the kit as it is now: its sound, trim, level, tune, filter, envelope, everything. **Store B** keeps a second version. Make A, change some pads (tune the snare down, close the hats' filter, push the kick), then make B.
- The **crossfader** (touch or drag it; it moves once A and B are both stored) morphs the kit from A to B as it plays: each pad's level, pan, tune, filter, envelope times, LFO depth and pitch envelope move between their values in A and in B. Settings that are the same in both scenes stay as they are. Sliding it changes nothing in the kit itself, so you can perform with it freely.
- **Return to A** puts every pad back exactly as A stored it and sets the crossfader to A. Mangle the kit as much as you like, then return. **UNDO** takes the return back.

The scenes are saved with the kit.

## Grain: stretch and smear

{{shot: sample-grain | The Grain page: tiny grains of the sound play at once, around the position you pick.}}

**Granular** playback plays the sound as a cloud of tiny overlapping pieces (grains). It can freeze a moment, stretch it into a pad, or scatter it into texture. Touch the square (**Touch**, under the picture) to move the **position** in the sound and the **size** of the grains together, and use the Q-Links for **position**, **size**, **density** and **pitch**. The four buttons over the picture pick what the panel beside the square holds, as on the Play page, and each one's second line says how its section is set, so you can see the whole pad without opening them:

- **Grains**: how the grains scatter and their shape. **Cloud** (the **Scan** knob moves through the sound on its own; **Spray**, **Size spray**, **Pitch spray**, **Level spray**, **Reverse** and **Stereo** scatter the grains' places, lengths, pitches, levels, directions and sides, and **Spray shape** how; **Timing** how evenly they come; **Level**), and **Shape** (**Mode**, **Grain mode**, the grains' **Shape**, **Pitch quantise** and **Tempo sync**). In a grain mode other than Cloud, the first section takes the mode's name and shows only the controls that mode uses.
- **Movement**: the **LFO** and the **envelope**, which move the cloud as it plays (see Movement below).
- **Filter and keys**: the filter on each grain; what the keys do to a cloud, **Size follows key**, **Scan follows key** and **Glide**; and **Quality**, the cloud's colour on the way out (see Filter, Playing a cloud from keys and Quality below).
- **Presets**: eight starting points, one tap each (see Presets below).

The square, the picture and the Q-Links stay as they are on all four. Touch a control and turn the data wheel to change it, or drag a knob up or down. The wheel walks through the controls of the section that is open.

The picture shows what the grains do as the pad plays. Each grain is a short bar where it is reading (cyan when it plays backwards), and the shaded band is where new grains can start: it widens and narrows as **Spray** and anything moving it change, and a peaked spray shades its middle darker, where most grains land. The thin line is where the pad is reading now, with a trail along the top showing where it went over the last second and an arrow the way **Scan speed** moves it. When the cloud wants more grains at once than a pad plays (64), a **thinned** tag lights up: the pad spaces them out to fit and keeps its level. If the texture sounds thinner than you want, shorten the grains or lower the density until the tag goes out.

Grains play only when the pad's **Mode** is **Granular**. Until then a button over the picture says which mode the pad is in: tap it to play the pad as grains. The **Mode** selector steps through every mode to **Granular** too (on the way, **Loop** gets a loop region of its own, as on the Wave page).

**Grain modes.** **Grain mode** picks what kind of granular sound the pad makes, and starts it from settings that work. **Cloud** scatters grains around the position (a little spray, spread and random timing) and shows every control. **Stretch** plays through the sound at its own speed with smooth, regular grains, so **Scan speed** slows it down or freezes it (0) without changing its pitch, and **Pitch** changes the pitch without changing its speed; only the controls a stretch needs are shown. If the sound has a tempo (detected or typed on the Warp page), a stretch plays in time: at **Scan speed** 1× it follows the project tempo ("1× · in time"), 0.5× at half the tempo, and it keeps following when the tempo changes. A stretch keeps drums punchy: each hit in the sound (where a drum or a note starts, found the first time the pad plays in Stretch) plays once, at its time and with its own attack, instead of smeared or doubled by the overlapping grains. Slowed down a lot, a drum loop then has short gaps before its hits, as a beat-slicing stretch does. **Repeat** replays the spot under the position every sixteenth note, a sixteenth long, locked to the tempo: a stutter. Its **Density** sets how often (from one a bar to 32 a beat) and **Size** how long each repeat is. **Tone** turns the sound into an oscillator: very short grains of one spot, so close together that their rate is the note you hear. **Density** shows that note (A2, 110 Hz, to start) and steps a semitone at a time, from A0 to B5; **Size** shows how many cycles of the note each grain holds (4 to start: fewer is buzzier, more is smoother); **Position** and **Pitch** pick which part of the sound it reads and how high, which changes its colour, not its note. Played in Keys mode, or with **Tune**, the keys move the note. Picking a mode puts in its own settings, keeping the position and the level; undo brings the old ones back. Projects from before grain modes open as clouds, with their settings as they were.

{{shot: sample-grain-presets | The Presets section: Shimmer fifths put on the pad and lit, its grains an octave up and scattered over fifths.}}

**Presets.** The **Presets** section puts a starting point on the pad in one tap: its grain mode and every grain control, its key and glide settings and its movement. Your sound, where the grains read (**Position**) and the **Level** stay as they are, so a preset works on any sound and keeps its loudness. Picking one also sets the pad's **Mode** to **Granular**. One undo brings back your settings from before; turning the wheel steps through the presets too, and a run of steps is one undo step.
- **Glass pad**: an octave up, long soft grains that open out over two seconds.
- **Frozen choir**: one spot held still, detuned a little and drifting.
- **Tape stretch**: half speed at the sound's own pitch, with a tape's wow; keys move the speed and the pitch together.
- **Stutter 1/16**: short repeats every sixteenth, jumping to a nearby spot each beat.
- **Shimmer fifths**: grains an octave up scattered over fifths and octaves, wide.
- **Grit cloud**: short hard-edged grains, rough in size, level and direction.
- **Swarm**: a dense buzz of small grains all over, wandering.
- **Sub drone**: the sound as a low A (55 Hz) oscillator, its colour slowly moving; keys glide to each note.

The preset that is on stays lit, and the **Presets** button names it, until you change one of its controls; then nothing is lit and the button says "your own settings": from there the pad is yours.

**In time with the beat.** **Tempo sync** locks the grains to the project tempo: **Density** starts them on the beat grid (grains per beat, from one a bar to 32 a beat), **Size** makes each one a length in beats (1/64 of a beat to 4 beats), and **Both** does both. Change the tempo and a cloud that is playing follows at once. Turn it **Off** and the density and size come back as they were. Synced grains always land on the grid, whatever **Timing** says. A cloud plays at about the level of its sound however long or dense its grains are, so turning **Size** or **Density** doesn't make it jump, and it sounds the moment you hit the pad, even with long grains (for a slow swell, raise the Play page's **Attack**). Very long grains at a high density would need more grains than a pad plays at once, so the cloud spaces them out evenly instead and keeps its level. **Level** sets how loud the cloud plays (−24 to +12 dB): a sparse cloud, with gaps between its grains, is quieter than its sound.

**Timing.** **Timing** sets how evenly the grains come. At **Regular** they arrive in lockstep, which gives a frozen sound a buzz at the grain rate and makes its level rise and fall as you turn **Density**; that buzz can be what you want for a pitched, mechanical tone. Each step toward **Random** loosens the gaps between grains, so a cloud sounds smoother and its level holds as you turn **Density**, though a single held tone shimmers a little. New grain pads start at 30 % random.

**Variation.** A cloud sounds alive because no two grains are quite alike, and each way they differ has its own row. **Spray** scatters where grains start around the position; **Spray shape** decides how: **Even** spreads them across the whole range, **Peaked** keeps most of them close to the position with a few strays further out, for a focused sound that still moves. **Size spray** makes each grain's length different, at 100 % anywhere from half to twice the **Size**. **Pitch spray** detunes each grain, **Reverse chance** plays some backwards, **Stereo spread** places each one across the stereo field, and **Timing** (below) loosens when they come. **Level spray** makes some grains quieter than others, at 100 % anywhere down to 24 dB less, so a few stand out from the cloud; the cloud as a whole keeps its level. All of them start at 0 (Even for Spray shape), so older projects sound as they did.

**Shape.** **Shape** sets how each grain fades in and out, from hard-edged to plucked, in one turn. At **0 %** (Square) a grain starts and stops almost at once, a buzzy, clicky cloud; toward **40 %** the fades lengthen to a **Triangle**; at **60 %** (Hann, where new grain pads start) the grain swells and fades smoothly, the softest blend; at **80 %** (Swell) it rises slowly and stops short, like a sound played backwards; at **100 %** (Pluck) it starts at once and dies away, so each grain is a little hit. Between 80 % and 100 % the swell turns into the pluck, passing a smooth fade on the way. The cloud keeps its level whatever the shape. A pad from an older project shows its window by its old name (Tukey, Trapezoid, Percussive) until you turn **Shape**.

**Playing a cloud from keys.** In Keys mode, or with **Tune**, a key above the root raises the grains' pitch (on a Tone pad, its note). **Size follows key** also makes the grains that much shorter, so the whole sound moves up as a sampled sound would, its grain texture too; **Scan follows key** makes the scan that much faster, as a tape played faster, so a stretch or a scanning cloud moves through the sound quicker on higher keys. Both start **Off**. **Glide** slides to each new key over its time (Off, or 10 ms to 5 s): on a mono pad (**Polyphony** 1 on the Play page), a key played while another is held moves the cloud that is playing there, without starting it again, and letting go of the first key keeps it playing; with more voices, each new note starts from the last one and slides to its own.

{{shot: sample-grain-filter | Filter and keys: a resonant band pass on each grain, its cutoff sprayed so each grain has its own colour; the grains' size follows the keys, glide slides between them, and Quality is Tape.}}

**Filter.** Each grain can go through a filter of its own, set when it starts. **Filter** picks **Low pass** (darker), **Band pass** (a narrower slice of the sound) or **High pass** (thinner, glassier); **Off**, where new pads start, leaves the grains as they are. **Cutoff** sets where it bites, from 20 Hz to 20 kHz, a sixth of an octave a step, and **Resonance** how much it rings there: a little gives a low pass a vocal edge, a lot makes each grain whistle at the cutoff, and it never runs away. **Filter spray** gives each grain a cutoff of its own, up to three octaves either side, so a cloud sounds like many sounds at once; a Stretch pad doesn't show it, so its grains stay alike. Turn the **LFO to** Cutoff for a sweep each grain catches as it starts. Changing grain mode keeps the filter. The Grains effect has the same filter on its Texture page.

**Quality.** **Quality**, beside Glide, colours the whole cloud on its way out. **Clean**, where every pad starts, leaves it as it is. **Tape** plays it as if off a tape: its pitch wavers a little (a slow wow of about five cents and a faster flutter), loud peaks round off softly, and the very top is a touch duller. **Lo-fi** is cold and digital: the sound runs at about a quarter of the sample rate and 8 bits, so the highs fold back into hard, metallic tones and quiet tails step down in grains of their own. Both keep about the same loudness as Clean, and switching while the pad plays crossfades without a click. Changing grain mode or trying a preset keeps the quality. The Grains effect has the same choice in its Output section.

{{shot: sample-grain-movement | The Movement section: a triangle LFO drifts the position, and the envelope opens the grains' size out after each hit.}}

**Movement.** A static cloud soon sounds still; movement makes it breathe. Each grain pad has an **LFO** and an **envelope**, and each can move one control as the grains are made: **Position**, **Size**, **Density** (on a Tone pad, its **Note**), **Pitch**, **Spray** or **Cutoff** (the grains' filter). Pick it with **LFO to** or **Envelope to** (both start **Off**), and set how far with the **depth**: at +100 % the LFO moves Position across the whole sound, Size and Density up to 8 and 16 times either way, Pitch 12 semitones, Spray all the way and Cutoff four octaves; a minus depth moves the other way. Small depths are subtle: an LFO on a Tone pad's **Note** at a few percent is a vibrato. **LFO shape** picks the wave: **Sine**, **Triangle**, **Saw up**, **Saw down**, **Square**, **Hold** (a new random value each cycle, held, for stepped chaos) or **Drift** (a new random value each cycle, glided to, for slow wandering). **LFO rate** runs from 0.05 to 40 Hz, or with **LFO sync** on, once every so many beats of the project tempo (1/4 beat to 64 beats), following tempo changes. The envelope rises from nothing to full over its **Env attack**, falls over its **Env decay** to its **Env sustain** and holds there: for a cloud that opens up after you hit the pad, or one that starts wide and settles. Both start again with every hit, and they move each grain as it starts, so a grain already playing finishes as it began. Changing grain mode keeps them. The cloud keeps its level as they move.

**Stereo spread** places each grain somewhere between left and right. At 100 % the cloud is wide, yet it keeps its level and still sounds full when played in mono.

**Every layer its own cloud.** The Grain page (and the Wave, Warp and Chop pages) work on the layer picked on the Play page's layer map, and the top line says which ("· layer 2"). So a pad can play its sound plainly on soft hits and as a cloud on hard ones.

## FX: effects on one pad, or on the whole kit

**FX**, in the Sample tab's row of pages, shows the effects on the pad you are editing as a row of pedals under its sound.

- **The pedals.** Each effect on the pad's own channel is a pedal: its number, its name and what it is set to, an **On** switch, and its first three knobs. Four show at once; with more, **Effects ‹ ›** on the right moves along them. Along the foot of the sound, a line shows the way it goes: the sound, each effect in order (one switched off is faint), then **Drums**.
- **Turning them.** Touch a pedal to pick it: it is outlined, and **Q-Links 1 to 4** turn its four main knobs. Drag a knob up or down, or touch it and turn the data wheel; a finger resting on a knob lets the wheel turn it until you lift. SHIFT turns a Q-Link finer.
- **+ Add effect** opens the effect picker on the pad's channel, and **Whole chain** the **chain editor** (see [Mixing and effects](effects.md#adding-an-effect-the-chain-editor)) on it, with every control of every effect, their order, presets and modulation. Both open in the Sample tab's place, and **Back** comes back to the Sample tab.

- **The pad's own effects.** The pedals are the pad's own channel's, so an effect added there plays on that pad alone. Add **Perform FX** for stutters, beat repeats, tape stops and reverse on just that pad, in time with the tempo. Each of its 16 slots (**Slot 1** to **Slot 16**) is a page in the editor, with its effect, its length and a **Hold** switch: Hold on plays it, off lets it go.
- **The whole kit's effects.** In **Whole chain**, the switch at the top goes to the track the kit plays into (**Drums** for kit 1), so an effect there plays on the whole kit, like a bus effect on the SP-404. To make it a one-knob effect, put its knobs on a macro (see [Macros](effects.md#macros)), and that macro's Q-Link plays them all at once.

The effects belong to the pad's place in the kit, not to its sound: put another sound on the pad and it plays through the same effects. Kit 1's bank A pads (A1 to A16) always have a channel of their own in the mixer. Any other pad, on any bank of any kit, gets one when you give it an effect with **+ Add effect** or **Whole chain**: the mixer's Pads view then shows it by its pad and kit ("B5 · Kit 2"), playing into its kit's track. Once the pad has nothing on it again, it gives the channel back. There are 48 of these channels; with all of them in use, the FX page says so, and taking another pad's effects off frees one.

## Playing a kit on keys

A kit's pads can play notes instead of their own sounds, so a bank of sampled notes plays like a keyboard instrument. A pad that plays notes is a *keygroup*: it covers a range of keys and plays its sound higher or lower for each key away from its root note.

**Map to keys**, under the pads on the Pads tab, does this for the whole bank at once. MPFree finds each pad's note: from its keygroup if it is one already, else from a note in its name ("Pluck C3", "Bass_F#1"), else from the pitch it hears in the sound. It then spreads the pads over the keyboard, each one reaching halfway to its neighbours, and the pads play notes from the C at or below the lowest. A pad with no note to find is left as it was. One UNDO takes the whole mapping back.

Notes are named with middle C as C4. Some libraries call middle C "C3"; when a name and the pitch MPFree hears are the same note, it uses the octave it hears.

{{shot: sample-keys-mapped | The Play page's keyboard after Map to keys: five keygroups, with pad A5's zone picked.}}

While the pad is a keygroup, or the pads play notes, the Play page's Pitch and Pad sections show a keyboard across the page:

- Each keygroup is a bar over the keys it plays, in its pad's colour, with a dot on its root.
- Tap a bar to pick its pad. Drag the picked pad's ends to set its lowest and highest keys, and its dot to move the root.
- Press the keys to hear the kit. The line under the keys shows the 16 notes the pads play.

The Pitch section's Keys controls do the same with numbers: the Keys switch makes one pad a keygroup, Low key, High key and Root key set its range, and the Pad section's Pads play switches the pads between their own sounds and notes.

### Crossfades

Where one zone ends and the next begins, the sound can switch at a hard edge or blend from one sample into the other. The same goes for a pad's velocity layers, from soft to hard.

- **Key fade**, a stepper in the Pitch section, sets the blend for the whole kit: every keygroup fades in and out over that many notes, and where two zones meet they are moved to overlap by as much, so one fades out while the next fades in. The level stays even across the blend.
- **Vel fade**, in the Layers section, does the same for the picked pad's velocity layers. Round robins keep taking turns while their layers blend.
- Set either one back to Off and the edges go back to where they met. Each change is one UNDO.

The keyboard and the layer map draw each fade as a curve at the ends of its bar.

{{shot: sample-keys-crossfade | Key fade at 4 notes: the zones that met now overlap and blend over four keys.}}

### One note at a time: Mono, Legato and Glide

A bass line or a lead plays one note at a time, sliding from note to note. The Pitch section's Keys part does that for a pad with keys:

- **Mono**: the pad plays one note, the last key you play. Let go of it while an earlier key is still held and the pad goes back to that key.
- **Legato**: with Mono on, a key played while another is held moves the sounding note to the new pitch without starting it again. Its envelope and its place in the sound carry on, so a slow attack doesn't come in again. A key played on its own starts the sound as usual.
- **Glide**: how long the pitch takes to slide to the next note, from Off up to 2 s. Every interval takes that long, a semitone or an octave.
- **Glide on**: **Every note** slides into each note from the last one played, even after you let go. **Held keys** only slides when you play a key while another is held, so separate notes land on their pitch at once.

They work on a pad's keys, from MIDI or with the pads playing notes. A hit on the pad itself plays as before, and while Keys is off the four say so. A warped sound and a sliced pad keep their pitch, and a grain layer slides with its own Glide on the Grain page. The Pitch chip says **mono** or **legato** while Mono is on.

{{shot: sample-play-mono | The Pitch section of a pad with keys: Mono and Legato on, and a 200 ms glide on held keys.}}

> [!MPC]
> On an MPC a keygroup instrument is a program type of its own. In MPFree keygroups are pads of an ordinary kit, so any kit can become one with Map to keys.
