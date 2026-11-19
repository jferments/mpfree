# Drum synths: what makes a great one

**Status:** research, 2026-11-18 (synth library stage 1, wave 3 survey). Feeds the [Drums engine](../design/synths.md#drums-mpfree-synth-drums). The 808 bass drum and the 808 as a bass are covered in [Bass](bass.md). Plan: "Synth library: stock-take and research plan" (https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). Nothing here has run on the MPC.

Where a point comes from a source it is linked; where it is our own reading, arithmetic or a target we propose, it says so.

## 1. What makes a great one

- **Each drum is its own instrument, with its own few knobs.** Akai's DrumSynth, the bar the owner set, has eight drum types (Clap, Crash, HiHat, Kick, Perc, Ride, Snare, Tom), each with a Model and up to eight knobs "specifically chosen for each sound", plus Velocity, a second velocity amount to any one of those knobs, One-Shot and a Gain from −68 to +12 dB (MPC 3.6 user guide, DrumSynth, pp. 419 to 421).
- **The processing is part of the drum.** Every DrumSynth drum carries a transient shaper (attack, shape, sustain), a distortion (pre-shape, 0 to 60 dB of drive, threshold, high cut, mix), a four-band EQ and a compressor (1:1 to 100:1, 100 µs to 300 ms attack); the multi-drum version adds send delay, a diffuser and reverb (Hall, Stadium, Room, Abstract) (MPC 3.6 guide). Modern kicks and snares are shaped as much by these as by the voice (our reading).
- **The classic circuits, honestly.** The 808's bass drum is a bridged-T resonator struck by a pulse, with a pitch that starts more than an octave high for about 6 ms and then wanders between about 48 and 58 Hz over the first 300 ms ([Bass](bass.md#1-what-makes-a-great-one), from Werner, Abel and Smith, DAFx 2014). Its cymbal, open and closed hats and cowbell share one bank of six square oscillators (205.3, 304.4, 369.6 and 522.7 Hz fixed, two more trimmed to about 540 and 800 Hz), through band-passes near 3.44 and 7.1 kHz and high-passes, with an envelope and VCA per voice ([K. J. Werner, J. S. Abel, J. O. Smith, "The TR-808 cymbal: a physically-informed, circuit-bendable, digital model", ICMC-SMC 2014](https://speech.di.uoa.gr/ICMC-SMC-2014/images/VOL_2/1453.pdf)). The 909 made its kick, snare, clap and toms in analog circuits and played its hats, crash and ride from 6-bit samples; its snare has Tone and Snappy, its kick Attack and Decay, and the snare and clap share a noise source ([Roland TR-909, Wikipedia](https://en.wikipedia.org/wiki/Roland_TR-909)).
- **Velocity and accent change the tone.** A harder hit should be brighter, snappier and shorter-attacked, not just louder (our reading, as for [Keys](keys.md#1-what-makes-a-great-one)); the 909 accents individual steps (Wikipedia).
- **No machine gun.** Repeated hits at one velocity should differ a little, as analog voices do; rolls and Note Repeat expose it at once (our reading).
- **Tight, sample-exact timing and clean chokes.** A closed hat that cuts the open hat, a new hit that replaces the last without a click. MPFree's Drums already do both (3 ms fades, choke groups).

## 2. How musicians use it

| Use | What it needs | Genres (examples) |
|---|---|---|
| Classic machine kits | 808 and 909 behaviours, accent, tune per drum | Hip-hop, house, techno, electro, Miami bass |
| 808 bass and long kicks | Pitch envelope, long tuned decay, drive (see [Bass](bass.md)) | Trap, drill, hip-hop, reggaeton |
| Hard modern kicks | Click layer, body, sub, distortion, transient and compression | Hardstyle, techno, EDM |
| Snares and claps | Tone against noise, snappy, room or reverb tail | Every genre |
| Hats and cymbals | Metallic oscillator banks or noise, decay on velocity, open/closed chokes | Every genre |
| Percussion | Toms, congas, rims, cowbells, woodblocks, FM blips, tuned to the key | Afro, Latin, electro, IDM |
| Sound-design hits | Zaps, noise sweeps, FM metal | Electro, IDM, trailers |

## 3. Algorithms

- **Circuit-informed analog voices.** The 808 bass drum and cymbal papers above give the structures, the component values and the filters; the cymbal model ran at 4× oversampling, with its VCA aliasing judged perceptually insignificant and no alias-suppressed oscillators used (Werner, Abel and Smith 2014). Our reading: on the MPC, band-limited squares (PolyBLEP) at 1× are cheaper than 4× oversampling and should be measured against it.
- **Subtractive and additive one-shots** (built): a tuned body with a pitch sweep, a noise layer, a click, drive (MPFree's eleven models).
- **FM drums.** Chowning's drum (c/m 5:7, index 2 over 0.2 s) and wood drum (c/m 16:11, index 25 falling fast) are the starting recipes ([FM](fm.md#3-algorithms)); FM perc is a Drums model today.
- **Modal percussion.** A bank of decaying resonators per drum, tuned to a membrane's or bar's modes; Pluck's mallets already use a modal bank (our reading: toms, congas, bells and woodblocks fit it at a few dozen modes).
- **Samples for metal, as the 909 did.** Pure synthesis is the owner's choice, so the hats and cymbals stay synthetic; a short synthetic metal table played back like a sample is a middle way (our reading).
- **Per-drum processing.** A transient shaper, drive, EQ and compressor per drum, and shared sends: the `mpfree-fx` units already exist after the FX quality rounds; what is missing is carrying them per drum in a kit.
- **Physical membrane models** (finite-difference drums) are the high end; our reading is that they don't fit the MPC's budget at 32 hits and are out of scope.

## 4. What to measure

Proposed targets for stage 4 (ours, in brackets, until the baseline and listening confirm them).

| Id | Measure | Proposed target | Why |
|---|---|---|---|
| D1 | 808 kick shape | Pitch start, sigh and decay within [5] % of the circuit model at matching settings | The classic, honestly |
| D2 | Metal bank | The six frequencies within [0.5] % and the band-pass centres within [5] % of the 808 cymbal paper | Hats that sound like hats |
| D3 | Velocity to tone | Spectral centroid rises monotonically with velocity on every model; attack transient rises at least [6] dB from velocity 40 to 127 | Harder means brighter |
| D4 | Hit variation | Two hits at one velocity differ by [0.2 to 1] dB in level and [1 to 5] cents in pitch, unless the preset turns it off | No machine gun |
| D5 | Decay accuracy | Time to −60 dB within [10] % of the label | Honest knobs |
| D6 | Chokes and retrigger | No step above [−60] dB at a choke or a retrigger | Clean rolls |
| D7 | Level | Each kit's pads within [6] dB of its kick at full velocity; no pad clips at 32 simultaneous hits | Balanced kits |
| D8 | Timing | Every hit starts on its frame, any block size | Tight grooves |
| D9 | Aliasing | Worst alias under [−60] dB for the metal bank at 1× | Clean metal |
| D10 | Cost | 32 hits sounding under [50] % of the synth budget | Fast rolls and full kits |

## 5. Knobs and defaults

| Drum | Knobs players expect (DrumSynth's idea: a few chosen per drum) | Proposed defaults |
|---|---|---|
| Kick | Tune, Decay, Pitch sweep and time, Click, Body/Sub mix, Drive | 50 Hz, 400 ms, 3 octaves over 30 ms |
| Snare | Tune, Decay, Tone (body against wires), Snappy, Noise colour, Ring | 185 Hz, Snappy half |
| Clap | Bursts (count and spread), Decay, Tone, Room | Four bursts, short room |
| Hats | Tune (metal bank), Decay (closed and open), Tone, Noise against metal, Choke | Open choked by closed |
| Cymbals | Tune, Decay, Tone, Spread | Long decay |
| Toms and percussion | Tune, Decay, Pitch sweep, Model (membrane, wood, metal, FM) | Three toms a fourth apart |
| Every drum | Velocity to level and tone, Accent, Variation, Level, Pan, Choke group, One-shot or gated, Transient, Drive, EQ, Compressor | Effects off unless the kit uses them |
| Kit | Sends (delay, reverb), Master drive and compressor | Room reverb send low |

## 6. The presets players expect

A first canon of 24 kits (our reading), each with a dozen or more drums: 808-style, 909-style, 707-style bright, 606-style tight, CR-78-style vintage, Linn-style punchy, electro, Miami bass, trap (long 808, rolling hats), drill, boom bap (driven, dusty), lo-fi (soft, bit-reduced), house, deep house, techno (hard, driven), minimal (clicks and blips), hardstyle (distorted kick), drum and bass (tight breaks-style), afro and amapiano (log drums, shakers), Latin percussion, reggaeton, synth-pop, industrial (metal and noise), and an FM kit. Akai's own kit names and sounds are not copied.

## 7. Playing it on the MPC

- **The pads are the drums.** Each pad its drum, labelled with its sound (built), velocity and pressure live: pressure could open a hat or bend a tom while held (our reading).
- **Note Repeat and rolls** are where variation and clean retriggers are heard first.
- **Q-Links** take the shown drum's four chosen knobs (Tune, Decay, Tone, Snap today); per-drum knob choices as DrumSynth does would make them the right four for each drum.
- **Kit pages** that show the hit's shape and sweep (built: Hit, Sweep, Levels cards).

## 8. What goes wrong

- One set of knobs for every drum, so half of them do nothing on a hat.
- Velocity that only changes the level.
- The machine gun: identical repeats.
- Kicks that lack weight on big speakers or vanish on small ones (see [Bass](bass.md#8-what-goes-wrong)).
- Hats that alias or sound like white noise.
- Chokes and retriggers that click.
- Processing left to the player: a raw kick next to a commercial one sounds weak.

## What MPFree has today

- **Drums** (5 kits: Init, Boom, Tight, Grit, Electro), 19 Nov, wave 3: 16 pads, sixteen models (the first eleven, and the 808 kick, Conga, Woodblock, Bell and Shaker), each drum its own knobs and Q-Links (section 5's idea), accent, per-hit variation, one-shot or gated pads, choke groups, 32 hit slots, the hit drawn on the Synth tab. The 808 kick uses Bass's body (punch and sigh measured against the paper), the hats and cymbal the cymbal paper's six squares and bands, band-limited (alias −62 dB at 1×), and the toms, congas, woodblocks and bells Pluck's modal bank. Old kits keep the first engine's voices to the bit. Measured against section 4: [Synth quality](../design/synth-quality.md#wave-3-targets-drums).
- **Missing:** per-drum processing in the kit (wave 3's second drums thread), more kits, pressure on the pads, and the cost of 32 hits at once on the MPC (D10).

## Sources

- MPC Standalone OS 3.6 user guide, DrumSynth, pp. 419 to 421 (project files)
- K. J. Werner, J. S. Abel, J. O. Smith, "The TR-808 cymbal: a physically-informed, circuit-bendable, digital model", ICMC-SMC 2014: https://speech.di.uoa.gr/ICMC-SMC-2014/images/VOL_2/1453.pdf
- K. J. Werner, J. S. Abel, J. O. Smith, "A physically-informed, circuit-bendable, digital model of the Roland TR-808 bass drum circuit", DAFx 2014: https://dafx14.fau.de/papers/dafx14_kurt_james_werner_a_physically_informed,_ci.pdf
- Roland TR-909, Wikipedia: https://en.wikipedia.org/wiki/Roland_TR-909
- [Bass](bass.md), [FM](fm.md) and their sources
