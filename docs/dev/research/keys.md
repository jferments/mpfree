# Keys: electric piano, clav, electric grand and piano

**Status:** research, 2026-11-18 (synth library stage 1, wave 1). Extends [Electric pianos](electric-piano.md), which covers the tine and reed pianos in depth and feeds the [Keys engine](../design/synths.md#keys-mpfree-synth-keys); this page adds the clav, the electric grand, the acoustic piano, and what every keyboard instrument needs from MPFree around the engine (velocity, the sustain pedal, the effects players pair with it). Plan: "Synth library: stock-take and research plan" (https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). Nothing here has run on the MPC.

Where a point comes from a source it is linked; where it is our own reading, arithmetic or a target we propose, it says so.

## 1. What makes a great one

- **Velocity changes the tone, not just the level.** On a tine piano the bark grows with how hard the key is struck, because a harder hit swings the tine further across the pickup's curve ([Electric pianos](electric-piano.md)). On an acoustic piano the felt hammer stiffens as it is compressed (force ∝ compression^p, with p from 2.3 at C2 to 3 at C6), so a harder strike is brighter ([Bank et al., EURASIP JASP 2003](https://air.unimi.it/retrieve/dfa8b99e-d2be-748b-e053-3a05fe0a3a96/bank_jasp03.pdf)).
- **The register sets the decay.** Low notes ring for many seconds, high notes for under one; Keys already follows the measured slope (T60 ∝ f^-0.75).
- **The release is part of the instrument.** The damper's thump on a tine piano; on the clav, the key's release lets the speaking and silent parts of the string merge, so every note ends with a short pitch drop of about three semitones as the yarn damps it ([Gabrielli et al., EURASIP JASP 2013](https://link.springer.com/article/10.1186/1687-6180-2013-103)).
- **The pickup and the amp are the sound.** The clav's two magnetic pickups, used alone or summed in or out of phase, each comb-filter the string (on A♯2, notches at every fifth partial), and four tone switches (soft, medium, treble, brilliant) colour it (Gabrielli). The electric grand's piezo pickups on the harp, with no soundboard, give a piano attack in a body that "makes feedback harder" ([Yamaha CP-70](https://en.wikipedia.org/wiki/Yamaha_CP-70)).
- **The effects belong to the sound.** Tine pianos were played through the amp's tremolo (a pan in stereo amps, speed and intensity on the front panel), phasers for "body on slower songs and ballads", a stereo chorus for width, a wah often left still as a fixed mid-boost, and a guitar amp's overdrive and spring reverb ([fenderrhodes.com, effects](https://www.fenderrhodes.com/history/effects.html)).
- **An acoustic piano lives in its partials.** Stiff strings are inharmonic (fk = k·f0·√(1 + B·k²)), more audibly in the bass; two or three slightly mistuned strings per key beat and decay in two stages; the soundboard colours everything (Bank et al.).

## 2. How musicians use it

| Instrument | Jobs | Usually with |
|---|---|---|
| Tine piano | Chords and comping, ballads, soul and jazz voicings, lo-fi loops | Tremolo or pan, phaser, chorus, a little drive |
| Reed piano | Riffs and stabs with bite, rock and soul | Its own tremolo, drive |
| FM piano | Bright, glassy chords that cut through a mix | Chorus, reverb |
| Clav | Percussive riffs and syncopated funk lines, often muted | Wah (moving or fixed), phaser, drive |
| Electric grand | Big piano chords with a bright edge, pop and rock ballads | Chorus, EQ |
| Acoustic piano | Everything: chords, melodies, bass lines | Reverb |

## 3. Algorithms

- **Tine and reed** (built): a small modal bank per voice, a hammer contact, and a nonlinear pickup curve followed by its derivative ([Electric pianos](electric-piano.md)).
- **FM piano** (built): FM's Glass Keys preset; more in the FM survey.
- **Clav** (Gabrielli et al. 2013): a digital waveguide string (fractional delay, loss filter, a "ripple" filter so partial decays vary, a dispersion filter, beating equalisers on the first partials), excited once by a smooth pulse fitted with a sixth-order polynomial and scaled by velocity; a second delay line switched in at release for the pitch drop; the pickup as a comb (gain −1, the reflection at the string's end) around a fourth-degree polynomial for the magnetic nonlinearity, with integration before it and differentiation after; the tone stack as IIR filters from circuit simulation; a sampled "knock" at note-on. Cost: ten strings and the amplifier ran at 97 % of a BeagleBoard in Pure Data at 44.1 kHz with 128-sample blocks; each string needs about 1,000 samples of memory, and the authors expect native code to cost much less. Our reading: on the MPC's Cortex-A17 in Rust this is affordable at 8 to 10 voices, and the knock can be a short synthetic burst instead of a sample.
- **Electric grand:** no paper found yet; its parts are known (hammer action, short strings, piezo pickups on the harp, no soundboard, 73 keys on the CP-70; [Wikipedia](https://en.wikipedia.org/wiki/Yamaha_CP-70)). Our reading: the piano model below with the soundboard left out and a bridge-force (piezo) output instead of radiated sound, which is cheaper than a full piano.
- **Acoustic piano** (Bank et al. 2003, a research overview): a digital waveguide string per string with a dispersion all-pass (order up to 20) for inharmonicity; the lowest two or three octaves at half the rate; the hammer as a mass on a nonlinear spring, run at twice the string rate for stability; beating and double decay from a bank of 5 to 10 resonators at 1/8 or 1/16 of the rate ("less than five multiplications per sample" per note); a multirate soundboard (split at 2.2 kHz, the low band down-sampled by 8) at about 100 operations a sample instead of a 2000-tap filter. Our reading: a full grand is the heaviest instrument in the library; it fits only as a "light" model with few strings per note, or through the sampler if a sampled piano is ever in scope (open-source modelled pianos are listed in [Electric pianos](electric-piano.md#open-source-pianos)).

## 4. What to measure

Proposed targets for stage 4 (ours, in brackets, until the baseline and listening confirm them).

| Id | Measure | Proposed target | Why |
|---|---|---|---|
| K1 | Brightness against velocity | Second-harmonic level rises at least [12] dB from velocity 40 to 127 on a tine preset; spectral centroid rises monotonically | Velocity changes tone |
| K2 | Level against velocity | Follows a square-law curve over the preset's dynamic range within [2] dB, the mapping [Dannenberg (ICMC 2006)](https://csound.com/manual/opcodes/ampmidid/) proposed; range [30 to 40] dB set per preset | Even, playable dynamics |
| K3 | Decay against pitch | T60 within [20] % of the preset's law across C1 to C7 | Register |
| K4 | Release | Damper sound present and under the note's level; clav pitch drop of [3 ± 0.5] semitones | The end of a note |
| K5 | Sustain pedal | Notes held while it is down end with their release when it lifts; a key restruck while held re-strikes | Keyboard players expect it |
| K6 | Key-to-key evenness | Loudness spread under [3] dB at a fixed velocity across the range | No holes |
| K7 | Clav pickups | Comb notches where the pickup position predicts, within [5] % | Pickup selector means something |
| K8 | Piano inharmonicity | Partial frequencies within [3] cents of fk = k·f0·√(1 + B·k²) for the first [20] partials | Piano-ness |
| K9 | Tremolo, phaser, chorus | Rates and depths as labelled (the FX movement and modulation measures) | Honest controls |
| K10 | Cost | 16 voices of tine under the synth budget; clav and electric grand at 10 voices under it | Fit the MPC |

## 5. Knobs and defaults

| Instrument | Controls players expect | Proposed defaults |
|---|---|---|
| Tine and reed (built) | Model, Hardness (bark), Bell, Decay, Release, Tone, Tremolo (rate, depth, pan or amp), Drive, Key noise | As Keys today |
| Clav | Pickup (neck, bridge, both, both out of phase), Tone (soft, medium, treble, brilliant), Mute (shorter decay), Release drop, Brightness | Both pickups in phase, Medium, no mute |
| Electric grand | Hammer (brightness), Decay, Detune between strings, Pickup tone, Stereo width | Moderate brightness |
| Piano | Hammer hardness, Decay, Unison detune, Soundboard, Release noise, Pedal noise | Neutral |
| All keys | Phaser, Chorus, Wah, Amp drive, Reverb saved with the preset; Velocity range | Off unless the preset uses them |

## 6. The presets players expect

A first canon of 32 (our reading): clean tine, bright tine, barking tine, mellow ballad tine, tine through phaser, tine with pan tremolo, tine with chorus, lo-fi tine (with the FX tab's lo-fi); reed, reed with bite, reed with tremolo; FM glass piano, FM bell piano; clav bright, clav muted, clav through wah, clav through phaser, out-of-phase clav; electric grand, electric grand with chorus; piano bright, piano soft, upright-ish piano, felt piano (soft hammers, close); and a few pressure presets (tremolo depth or wah on press).

## 7. Playing it on the MPC

- **Velocity.** Pads give a different spread of velocities from keys, so each preset's velocity range (K2) and the pads' velocity curve need tuning together (our reading); Full and Half Level still apply.
- **The sustain pedal.** MPFree's MIDI monitor reads CC 64 (`mpfree-midi` `monitor::now`), and profiles name it, but no instrument hears it: the node events are NoteOn, NoteOff, Pressure, Bend, Expression, Param and AllOff. A sustain event (or the host holding note-offs) is an architecture item for stage 3; a piano or tine model that knows the pedal can also let undamped strings ring in sympathy (Bank et al. model pedal coupling with a common termination).
- **Pressure.** Tremolo depth, wah, or the amp's drive.
- **Q-Links.** Tone, Bark, Tremolo, Effects mix for tines; Pickup, Tone, Mute, Wah for clav.

## 8. What goes wrong

- Velocity that only changes the level, so soft and hard notes sound like one sample at two volumes.
- Repeated notes that sound identical ("machine gun"), which Keys already avoids with its per-strike variation.
- No release sound, or a release that clicks.
- Effects that sound sterile next to the amp, phaser and chorus players know (the FX rounds' movement and modulation work applies).
- No sustain pedal.
- Too few voices, so a held chord with the pedal cuts off.

## What MPFree has today

- **Keys:** tine, reed and (from 18 Nov) clav, 32 presets, a Velocity range per preset, 12 voices by default (16 at most), never played on the MPC. See [Keys](../design/synths.md#keys-mpfree-synth-keys).
- **FM:** Glass Keys and other FM piano sounds.
- **Effects** that suit keys, already in `mpfree-fx` after the FX quality rounds: chorus, phaser, auto wah, tremolo and auto-pan, drive, amp, spring reverb. Presets can't carry them yet.
- **Missing:** electric grand, acoustic piano; the sustain pedal reaching instruments.

## Sources

- B. Bank, F. Avanzini, G. Borin, G. De Poli, F. Fontana, D. Rocchesso, "Physically informed signal processing methods for piano sound synthesis: a research overview", EURASIP JASP 2003: https://air.unimi.it/retrieve/dfa8b99e-d2be-748b-e053-3a05fe0a3a96/bank_jasp03.pdf
- L. Gabrielli et al., a real-time emulation of the Clavinet, EURASIP JASP 2013, article 103: https://link.springer.com/article/10.1186/1687-6180-2013-103
- Yamaha CP-70: https://en.wikipedia.org/wiki/Yamaha_CP-70
- Fender Rhodes Super Site, effects: https://www.fenderrhodes.com/history/effects.html
- R. Dannenberg, "The interpretation of MIDI velocity", ICMC 2006, as implemented in Csound's `ampmidid`: https://csound.com/manual/opcodes/ampmidid/
- [Electric pianos](electric-piano.md) and its sources
