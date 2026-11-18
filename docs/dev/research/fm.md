# FM synths: what makes a great one

**Status:** research, 2026-11-18 (synth library stage 1, wave 2 survey). Feeds the [FM engine](../design/synths.md#fm-mpfree-synth-fm), a six-operator synth in the DX7's mould. Plan: "Synth library: stock-take and research plan" (https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). Nothing here has run on the MPC.

Where a point comes from a source it is linked; where it is our own reading, arithmetic or a target we propose, it says so.

## 1. What makes a great one

- **Spectra that move with the note.** FM's whole point is a spectrum set by two numbers: the carrier-to-modulator ratio places the partials and the modulation index decides how many are strong. Integer ratios c/m = N1/N2 give harmonic spectra (N2 = 1 all harmonics, N2 even odd harmonics only, N2 = 3 every third missing); irrational ratios such as 1/√2 give bells and metal ([J. Chowning, "The synthesis of complex audio spectra by means of frequency modulation", JAES 1973](https://web.eecs.umich.edu/%7Efessler/course/100/misc/chowning-73-tso.pdf)). Sweeping the index in time is what makes it an instrument.
- **The index follows the loudness.** Chowning's brass is a 1:1 pair whose index rises from 0 to 5 with the amplitude envelope, so it gets brighter as it gets louder; his bell (c/m 5:7, index 10, 15 s) lets the index fall with an exponential decay, so it ends near a sine (Chowning 1973). He describes it as "just by using the intensity envelope as a modulation index", after Risset's trumpet analysis ([Chowning interview](https://homepages.abdn.ac.uk/d.j.benson/pages/html/Chowning.html)). On a DX7 that is each modulator's envelope and its velocity sensitivity: harder notes, brighter tones.
- **High notes that stay clean.** Scaling the index down with pitch keeps high notes near sines, which also keeps them from aliasing (Chowning interview). The DX7's keyboard level scaling does this per operator.
- **Feedback that grits without exploding.** The DX7 feeds an operator's last two outputs back, averaged, to keep the loop stable ([K. Shirriff, reverse-engineering the DX7's sound chip](https://www.righto.com/2021/11/reverse-engineering-yamaha-dx7.html)). MPFree's FM does the same.
- **The DX7's grain.** It is phase modulation (FM with the modulator's derivative), with a log-sine ROM and exponential ROMs so that every multiplication is an addition, 16 notes × 6 operators time-shared through one circuit at about 49.1 kHz (Shirriff). Our reading: the "DX sound" players want is mostly the envelopes, scaling and algorithms, which MPFree models; the chip's own word lengths are a later, optional colour.
- **Approachable, not a spreadsheet.** FM has a reputation for being hard to program. The Digitone shows one answer: four operators, eight algorithms built from a few recognisable stacks and branches, and two outputs with an X/Y crossfade that changes the sound "from a single algorithm of only four operators" ([Perfect Circuit, FM on the Digitone](https://www.perfectcircuit.com/signal/fm-synthesis-elektron-digitone)). Our reading: a macro layer over the six operators (ratio, harmonics, detune, feedback, brightness, decay) makes the same engine playable without hiding the full panel.

## 2. How musicians use it

| Use | Typical recipe | Genres (examples) |
|---|---|---|
| Electric piano | Two stacks, a bright short modulator on the tine | Ballads, R&B, city pop, gospel, 1980s pop |
| Bells, chimes, mallets | Non-integer ratios, index decaying with the amplitude | Pop, ambient, game music, film |
| Bass | 1:1 or 1:2 with a fast index decay and feedback; slap and rubber basses | Synth-pop, funk, house, techno |
| Brass and leads | A single pair with feedback on the modulator, index on velocity ("Most brass instruments can be modeled with a single FM pair, with feedback on the modulator", [T. Zollinger, FM synthesis of real instruments, 2016](https://csclub.uwaterloo.ca/~pbarfuss/FM_Synthesis_of_Real_Instruments.pdf)) | Synth-pop, soundtrack |
| Woodwinds | Odd-harmonic ratios (Chowning's clarinet 3:2, bassoon 5:1); Zollinger's clarinet as carriers at 1, 3, 5, 7 | Soundtrack, world, lo-fi |
| Pads and glass | Detuned carriers that beat, slow index envelopes | Ambient, pop |
| Percussion | Short wide index bursts: Chowning's wood drum (c/m 16:11, index 25 falling fast), log drums | Dancehall, afro, techno |
| Metallic and noisy | High feedback, inharmonic ratios, fast index sweeps | Techno, IDM, sound design |

## 3. Algorithms

- **Phase modulation with a sine table** (built): six operators, the DX7's 32 algorithms, feedback averaged over two samples, envelopes in decibels at constant rates, keyboard level and rate scaling ([FM](../design/synths.md#fm-mpfree-synth-fm)).
- **Fewer operators with a macro layer.** Four operators and a handful of algorithms (Digitone); or the six-operator engine shown through macros. Our reading: macros that move several operators at once (all modulators' levels as "Brightness", every ratio's fine tune as "Detune") cost nothing on the audio thread, since FM's `set_param` only reworks what a knob feeds (0.07 to 0.15 µs a turn).
- **Other operator waves.** Current FM synths let operators use shapes other than the sine (Perfect Circuit). Our reading: a few half-sine and pulse-like shapes from one table each, band-limited by key, cover the common uses.
- **Aliasing control.** Index falls with key (keyboard scaling, built); 2x oversampling for high feedback or high index at high notes is the fallback (our reading; the DX7 ran at about 49.1 kHz, a little above ours).
- **Linear through-zero FM** between oscillators of the subtractive engines (Wavetable's FM warp is one) is a different tool: a sound-design colour, not an instrument model (our reading).
- **Recipes as data.** Chowning's table of instruments (brass 1:1 index 0 to 5; woodwind 3:1 index 0 to 2; bassoon 5:1 index 0 to 1.5; clarinet 3:2 index 4 to 2 inverse to the amplitude; bell 5:7 index 10; drum 5:7 index 2; wood drum 16:11 index 25) and Zollinger's operator settings are starting points for the preset canon, rewritten in MPFree's own parameters.

## 4. What to measure

Proposed targets for stage 4 (ours, in brackets, until the baseline and listening confirm them).

| Id | Measure | Proposed target | Why |
|---|---|---|---|
| F1 | Sidebands | A single pair's partials within [0.5] dB of the Bessel amplitudes Jn(I), for I from 0 to [10] | The theory holds |
| F2 | Harmonicity | Integer ratios: no partial off the harmonic series above [−80] dB | Clean harmonic tones |
| F3 | Aliasing | Worst alias under [−60] dB at C6 with every preset; under [−50] dB at C7 | No fizz on top notes |
| F4 | Envelope rates | Times for DX7 rates and levels within [5] % of the published formula | Imported banks behave |
| F5 | Feedback | Feedback 7 stays bounded and its level within [3] dB of feedback 0 | Grit, not explosions |
| F6 | Banks | Every voice of a 32-voice bank imports; write-back byte-exact | Players' own banks |
| F7 | Level across algorithms | Within [3] dB for one preset under every algorithm | No jumps |
| F8 | Velocity | Spectral centroid rises monotonically with velocity on every preset that uses it | Harder means brighter |
| F9 | Cost | 16 voices of six operators under [50] % of the synth budget | EP chords with the pedal |
| F10 | Knob speed | A knob turn under [1] µs (0.15 µs today on the desktop) | Macros and locks move many |

## 5. Knobs and defaults

| Layer | Controls players expect | Proposed defaults |
|---|---|---|
| Macros (first page) | Brightness (modulator levels), Harmonics or Ratio (step through useful ratios), Detune, Feedback, Attack, Decay, and an X/Y-style carrier mix where the algorithm allows | Per preset |
| Algorithm | Algorithm (32), Feedback 0 to 7, Transpose, Level, Voices | 1:1 pair, feedback 0 |
| Operators | Ratio or fixed, Fine, Detune, Level 0 to 99, envelope (four rates and levels), Velocity sensitivity, keyboard scaling, rate scaling, AM | DX7 init voice |
| Modulation | LFO (six shapes, speed, delay, pitch and amp depth, key sync), pitch envelope | Off |
| Expression | Pressure to brightness, vibrato or level; mod wheel to LFO | Pressure to brightness |
| Effects | Chorus, delay, reverb saved with the preset | Chorus on EP and pad presets |

## 6. The presets players expect

A first canon of 40 (our reading): FM electric pianos (bright, soft, bell-tine, chorus), glass keys, harpsichord-ish, clav-ish; tubular bells, church bell, glockenspiel, celesta, music box, marimba, vibes, kalimba; rubber bass, slap bass, solid bass, wood bass, sub-FM bass; brass section, synth brass, trumpet-ish lead, French-horn pad; clarinet, oboe-ish, flute-ish pad; silk pad, glass pad, choir-ish pad, sweep pad; drawbar organ, perc organ; steel pluck, koto-ish pluck, harp-ish; log drum, FM toms, metal hit, noise sweep. Players' own DX7 banks load as they are; MPFree ships no Yamaha voices.

## 7. Playing it on the MPC

- **Macros on the Q-Links**, the full operator pages behind them.
- **The algorithm drawn**, as the FM pages already do (carriers green, modulators blue, the feedback loop shown); a tap opens an operator.
- **Pressure** to brightness (every modulator's level at once) is the most FM-like gesture: a held chord that opens up under the finger.
- **Velocity to index**, tuned against the pads' velocity spread (see [Keys](keys.md#7-playing-it-on-the-mpc)).

## 8. What goes wrong

- Presets that sound like a 1980s demo and nothing else: the canon needs modern basses, pads and percussion too.
- Fizz on high notes (no index scaling, high feedback).
- A wall of 150 parameters and no way in.
- Volume jumps when the algorithm or the number of carriers changes (handled today with 1/√carriers).
- Feedback that screams or clicks at high settings.
- Imported banks that play differently from the original (envelope rates, scaling curves).

## What MPFree has today

- **FM** (`mpfree-synth`, 9 presets, 1 to 16 voices): six operators, 32 algorithms, DX7-style envelopes and scaling, an LFO and pitch envelope, pressure through the shared shaper, .syx bank import and export, and the algorithm pages with drawn envelopes and scaling. It plays on the pads.
- **Missing:** a macro layer, operator shapes other than the sine, a modern preset canon beyond the 9, effects saved in presets, cost measured on the MPC.

## Sources

- J. Chowning, "The synthesis of complex audio spectra by means of frequency modulation", JAES 21(7), 1973: https://web.eecs.umich.edu/%7Efessler/course/100/misc/chowning-73-tso.pdf
- John Chowning, interview: https://homepages.abdn.ac.uk/d.j.benson/pages/html/Chowning.html
- K. Shirriff, "Reverse-engineering the Yamaha DX7 synthesizer's sound chip from die photos", 2021: https://www.righto.com/2021/11/reverse-engineering-yamaha-dx7.html
- T. Zollinger, "FM synthesis of real instruments", 2016: https://csclub.uwaterloo.ca/~pbarfuss/FM_Synthesis_of_Real_Instruments.pdf
- Perfect Circuit, "FM synthesis and the Elektron Digitone": https://www.perfectcircuit.com/signal/fm-synthesis-elektron-digitone
