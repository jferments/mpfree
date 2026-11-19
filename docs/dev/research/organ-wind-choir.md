# Organs, winds, choirs and tape: what makes a great one

**Status:** research, 2026-11-18 (synth library stage 1, wave 2 survey). Covers the plan's Organ, Wind and Choir type: combo and pipe organs (the tonewheel organ already has the [Drawbar engine](../design/synths.md#drawbar-mpfree-synth-drawbar) and a deep page, [Tonewheel organs and rotary speakers](tonewheel-organ.md)), wind instruments, choirs, and the tape-replay keyboard's character. Plan: "Synth library: stock-take and research plan" (https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). Nothing here has run on the MPC.

Where a point comes from a source it is linked; where it is our own reading, arithmetic or a target we propose, it says so. The owner chose pure synthesis; sampled instruments from the MPC's own factory content are for later tools that owners run on their own devices, and nothing on this page depends on them.

## 1. What makes a great one

- **Tonewheel organ** (built, see [Tonewheel organs](tonewheel-organ.md)): wheels always turning, drawbars, key click, percussion, scanner vibrato, overdrive and the rotating speaker.
- **Combo organ: cutting and thin, on purpose.** The Vox Continental used 12 transistor oscillators for the top octave, each divided by two for every octave below; six drawbars (16', 8', 4' and a higher "IV" mixture on the left, flute and reed voices on the right; 5 1/3' on the upper manual of the two-manual model) and one vibrato with one speed and one depth. Players describe a "drier, thinner, more cutting sound" ([Vox Continental, Wikipedia](https://en.wikipedia.org/wiki/Vox_Continental)). A restorer's notes on the single-manual model say the flute voice ("~") is close to a sine, the reed voice ("M") close to a sawtooth, and pulling both all the way in gives no sound at all; each divider's own filter makes a saw, and one filter for the whole keyboard rounds the flute toward a sine; the vibrato has one fixed speed; there is no percussion and no sustain ([Vox Continental, combo-organ.com](https://combo-organ.com/Vox/Single/index.htm)). The Farfisa Compact starts instead from a pulse about two thirds high, filtered toward a sine, with a booster that adds back a brighter, high-passed pulse ([Farfisa Compact, combo-organ.com](https://combo-organ.com/Farfisa/Compact/index.htm)). That is the string machine's divide-down bank with different filters (see [String machines](string-machine.md#3-algorithms)).
- **Pipe organ: ranks, wind and a room.** Stops are ranks of pipes at fixed footages; tuning in historical temperaments; a long church reverb. Aeolus synthesises one rather than sampling it: three manuals and a pedal, five temperaments, variable tuning, and "a large church reverb" ([Aeolus, Linux audio wiki](https://wiki.linuxaudio.org/apps/all/aeolus)). Its licence must be checked before reading its code.
- **Winds: the breath is the instrument.** A wind model plays from a continuous breath pressure, with reed stiffness, breath noise and vibrato as its other controls (the STK clarinet by Perry Cook and Gary Scavone, as [Csound's STKClarinet](https://csound.com/manual/STKClarinet.html) exposes it). Without continuous control a wind model is an organ with a slow attack (our reading).
- **Choirs: vowels and many singers.** A sung vowel is a set of formants, each a frequency, bandwidth and amplitude; for a bass singing "a": 600, 1040, 2250, 2450 and 2750 Hz, bandwidths 60 to 130 Hz, amplitudes 1, 0.45, 0.35, 0.35 and 0.1 ([Csound's vowel tables via `vowelsdb`](https://csound-plugins.github.io/csound-plugins/opcodes/vowelsdb.html)). IRCAM's CHANT adds rules that move the formants with the sung pitch and the dynamics (AUTOBEND, CSLOPE) ([OM-Chant, formants](https://support.ircam.fr/docs/om-libraries/om-chant/co/Formants.html)). A choir is many such voices, each with its own small detune and vibrato, which is the ensemble problem again (our reading).
- **Tape replay: every note slightly different.** A Mellotron key pulls a length of tape across a head and a spring pulls it back on release; a harder press pushes the head harder; tape wow and flutter make each note waver; many keys at once drag the motor flat ("Tuning a Mellotron doesn't", Robert Fripp); each frame held three sounds on the M400; the choir was recorded with four men and four women ([Mellotron, Wikipedia](https://en.wikipedia.org/wiki/Mellotron)). A note lasts only as long as its tape; about eight seconds is the figure usually quoted, which we have not confirmed from a primary source yet.

## 2. How musicians use it

| Instrument | Jobs | Genres (examples) |
|---|---|---|
| Tonewheel organ | Comping, pads, bass on the pedals, screaming leads through the speaker | Gospel, soul, jazz, rock, reggae |
| Combo organ | Stabs and riffs that cut through guitars | 1960s garage and pop, ska, new wave (Wikipedia lists the Animals, the Doors, OMD) |
| Pipe organ | Chorales, drones, big chords | Film, ambient, gothic, hymns |
| Winds | Lead lines, doubled melodies, lo-fi flute loops | Soundtrack, lo-fi hip-hop, world, jazz-ish leads |
| Choir | Pads ("ahh", "ooh"), swells, stabs | Film, gospel, trance, ambient, pop |
| Tape character | Flutes, strings and choirs that waver | Psychedelia, prog, indie, lo-fi hip-hop (Wikipedia lists the Beatles, the Moody Blues, Genesis, Radiohead, Air) |

## 3. Algorithms

- **Tonewheel** (built): 91 always-turning wheels, cost per wheel not per key ([Drawbar](../design/synths.md#drawbar-mpfree-synth-drawbar)).
- **Combo organ:** a divide-down bank (12 top-octave oscillators, dividers below), footage mixes and fixed voice filters (flute, reed), one vibrato. Our reading: the same bank as the string machine, so both cost the same whatever is held.
- **Pipe organ:** additive ranks with a short attack noise ("chiff") and slight random detune per pipe, temperaments, a long reverb (our reading of the type; Aeolus as a reference once its licence is known).
- **Winds, several ways:**
  - *Digital waveguide*: a bore as a delay line with a reflection filter, excited by a reed or jet nonlinearity driven by breath pressure (the STK clarinet and flute, Cook and Scavone). Our reading: one delay line and a few filters per voice, the cost of Pluck's string.
  - *FM*: Chowning's woodwinds (clarinet 3:2 with the index inverse to the amplitude, bassoon 5:1) and Zollinger's clarinet and saxophone recipes (see [FM](fm.md#2-how-musicians-use-it)).
  - *Subtractive*: a pulse or saw, a breath-noise layer and a filter that opens with pressure.
- **Choir, several ways:**
  - *Formant filter bank*: a buzzy source (a band-limited pulse with a spectral tilt) through five resonators per voice from the vowel table, morphing between vowels.
  - *FOF* (CHANT): each formant as a train of decaying sine grains, with rules that set bandwidths and amplitudes from the pitch and dynamics.
  - *Formant wavetables*: Wavetable's Vowels table is already one (A, E, I, O, U).
  - *Ensemble*: several detuned voices per note or the shared ensemble effect from [String machines](string-machine.md#3-algorithms).
- **Tape character** as an effect or a mode any engine can use (our reading): wow and flutter on pitch, a per-key level and tone offset, a note time limit, motor drag that flattens with keys held, tape hiss and head EQ.

## 4. What to measure

Proposed targets for stage 4 (ours, in brackets, until the baseline and listening confirm them).

| Id | Measure | Proposed target | Why |
|---|---|---|---|
| O1 | Formants | Peaks within [5] % of the vowel table, bandwidths within [20] % | Vowels you can name |
| O2 | Vowel morph | A full morph with no step above [−60] dB | Smooth "ooh" to "ahh" |
| O3 | Breath | Level and brightness rise monotonically with pressure; the note speaks within [30] ms at full breath | Winds that play |
| O4 | Wind tuning | Within [3] cents across the range (fractional delay) | In tune |
| O5 | Organ cost | Flat within [10] % from 1 to 49 keys held (as S5) | Divide-down promise |
| O6 | Combo footages | Octaves exact, within [0.1] cent | Divide-down truth |
| O7 | Tape | Wow and flutter rates and depths as labelled; a note stops at its time limit when the limit is on | Honest character |
| O8 | Choir ensemble | Each voice's vibrato decorrelated; stereo correlation [0.2 to 0.6] | Many singers, not one |
| O9 | Attack noises | Key click and chiff present at their labelled level, under the note's level | The start of a note |
| O10 | Cost | A 6-note choir chord of [4] voices each under [50] % of the synth budget | Fit the MPC |

## 5. Knobs and defaults

| Instrument | Controls players expect | Proposed defaults |
|---|---|---|
| Tonewheel (built) | Drawbars, click, percussion, vibrato and chorus, drive, rotary | As Drawbar today |
| Combo organ | Footages (16', 8', 4', mixture), Flute and Reed voices, Vibrato on and depth, Bass section | 8' and 4' with Reed, vibrato on |
| Pipe organ | Stops by footage (16', 8', 4', 2 2/3', 2', mixture), Chiff, Temperament, Room | 8' and 4', equal temperament, room on |
| Winds | Instrument (clarinet, flute, sax, brass), Breath source (pressure, velocity, CC 2), Reed or jet, Noise, Vibrato (rate, depth, delay), Brightness | Pressure as breath |
| Choir | Vowel (and morph), Voice type (soprano to bass), Singers per note, Detune, Vibrato, Breath noise, Attack, Release | "Ahh", alto, 3 singers, slow attack |
| Tape character | Wow, Flutter, Hiss, Time limit (on or off), Drag, Age | Off; on in the tape presets |

## 6. The presets players expect

A first canon of 32 (our reading): gospel organ, jazz organ, rock organ with drive, ballad organ (already among Drawbar's 12); combo organ bright, combo organ with vibrato, reed combo; pipe principal, full organ, flute stop, drone; clarinet, flute, breathy flute, alto sax-ish, trumpet-ish, French horn pad, brass section; choir "ahh", "ooh", boys' choir-ish, male choir, choir swell, vocal pad, vowel lead; tape flute, tape strings, tape choir, tape brass, worn tape (more wow and hiss); and organ or choir through the ensemble.

## 7. Playing it on the MPC

- **Pressure as breath.** Pad pressure is per note and already reaches every engine through the shared shaper; a wind preset uses it as breath, so a note swells and opens under the finger.
- **A breath controller.** MIDI CC 2 (breath) has no path to instruments yet: `mpfree-node`'s events are NoteOn, NoteOff, Pressure, Bend, Expression (per note: Slide, Tone, Release), Param and AllOff. A channel-wide breath or controller event is an architecture item for stage 3, alongside the sustain pedal (see [Keys](keys.md#7-playing-it-on-the-mpc)).
- **Q-Links** as drawbars (four at a time, as Drawbar's pages do), vowel and singers for choirs, breath and vibrato for winds.
- **Chord pads** for organ and choir pads.

## 8. What goes wrong

- A wind instrument with no continuous control, which plays like an organ with a slow attack.
- A choir that sounds like one singer through a filter (no ensemble), or vowels you can't tell apart.
- Organs that click when they shouldn't, or never click when they should.
- Tape character that is a cartoon (too much wow) or switched on in every preset.
- Cost that climbs with keys held for organs.

## What MPFree has today

- **Drawbar** (20 presets): tonewheel organ and rotary speaker, and since 19 Nov a **Combo** model on the Strings bank with flute and reed voices and 8 combo presets (see [Drawbar](../design/synths.md#drawbar-mpfree-synth-drawbar)); never played on the MPC.
- **Wavetable:** the Vowels factory table and the Vowel Lead preset; **FM:** Drawbars and Brass Section; **Pluck:** a waveguide string engine that a wind model could share code with.
- **Missing:** the pipe organ, the combo's bass section, a wind engine, a choir engine, tape character, a breath event reaching instruments.

## Sources

- Vox Continental, Wikipedia: https://en.wikipedia.org/wiki/Vox_Continental
- Vox Continental (single manual), combo-organ.com: https://combo-organ.com/Vox/Single/index.htm
- Farfisa Compact, combo-organ.com: https://combo-organ.com/Farfisa/Compact/index.htm
- Aeolus, Linux audio wiki: https://wiki.linuxaudio.org/apps/all/aeolus
- Csound, STKClarinet (from the STK by P. R. Cook and G. P. Scavone): https://csound.com/manual/STKClarinet.html
- csound-plugins, `vowelsdb` (Csound manual's formant tables and others): https://csound-plugins.github.io/csound-plugins/opcodes/vowelsdb.html
- IRCAM, OM-Chant, formants: https://support.ircam.fr/docs/om-libraries/om-chant/co/Formants.html
- Mellotron, Wikipedia: https://en.wikipedia.org/wiki/Mellotron
- [Tonewheel organs and rotary speakers](tonewheel-organ.md), [String machines](string-machine.md), [FM](fm.md) and their sources
