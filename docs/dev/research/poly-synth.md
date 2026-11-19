# Poly and wavetable synths: what makes a great one

**Status:** research, 2026-11-18 (synth library stage 1, wave 2 survey). Covers the polyphonic subtractive synth (the [Poly](../design/synths.md#poly-mpfree-synth) and [Analog](../design/synths.md#analog-mpfree-synth-analog) engines) and the [Wavetable](../design/wavetable.md) oscillator, which the plan folds into Poly as an oscillator type. The vintage analog side is covered in depth in [Vintage analog polysynths](vintage-poly.md); this page is the survey of the whole type. Plan: "Synth library: stock-take and research plan" (https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). Nothing here has run on the MPC.

Where a point comes from a source it is linked; where it is our own reading, arithmetic or a target we propose, it says so.

## 1. What makes a great one

- **Oscillators that don't fizz.** A saw or a square has harmonics far past half the sample rate; sampled naively they fold back as inharmonic noise that moves against the pitch ([Virtual analog oscillators, CCRMA](https://ccrma.stanford.edu/~juhan/vas.html)). High notes, fast pitch sweeps, sync and FM are where it shows.
- **Thickness that beats at many rates.** The supersaw stacks seven saws. At the JP-8000's maximum detune their relative frequencies are about 0.893, 0.939, 0.980, 1, 1.020, 1.064 and 1.110, so the outer pair sits about two semitones out; most players use 40 to 90 cents in total. Evenly spaced saws give one strong beat rate instead of the many that make the sound; a mix knob sets the centre saw against the sides; a high-pass that follows the key sits near the fundamental; and the oscillators must run free or start at random phases, "perhaps the most important thing" ([KVR thread, "Roland Supersaw"](https://www.kvraudio.com/forum/viewtopic.php?p=4644620)). Adam Szabo's thesis fits the detune knob's response with an 11th-order polynomial, which a later measurement reads as three straight segments ([atosynth, super saw analysis](https://atosynth.blogspot.com/2021/07/super-saw-analysis-ideas.html)).
- **A filter that sings when pushed.** The resonant low-pass is the voice of the type; overdriven ladders and OTA filters squash rather than run away (see [Bass](bass.md#3-algorithms) and [Vintage analog polysynths](vintage-poly.md#the-filter)).
- **Every voice a little different.** On analog polys "+/- 5-10 cents variation per voice is expected", and the filters differ too ([Vintage analog polysynths](vintage-poly.md#voices-and-why-no-two-are-alike)). Analog already models it; Poly is clean on purpose.
- **Movement under the player's hands.** A pad that doesn't move is a test tone. Wavetable synths make timbre a position to sweep (an envelope, an LFO, pressure); Akai's own two poly instruments show the two faces players expect: TubeSynth's full panel (two oscillators from Wide to 2', a sub with a continuous Triangle, Saw, Square, Pulse shape, envelopes from 1 ms to 100 s, a third envelope, two LFOs, a modulation matrix, Quad and Detune, Legato or Retrigger, a tube Output Shape and Width) and Hype's macro face (six macros per preset, plus Cutoff, Reso, Mod, Distortion, Delay, Reverb and a Pumper) (MPC 3.6 user guide, TubeSynth and Hype). Our reading: one engine should offer both, a macro page first and the full panel behind it.
- **Level that stays put.** A chord, more unison or more drive should not jump the volume. Poly already keeps level through drive (its `DriveLevel`) and scales unison by 1/√n.

## 2. How musicians use it

| Use | What it needs | Genres (no limits; examples) |
|---|---|---|
| Pads and beds | Slow attack, long release, chorus, movement (LFO, table position), wide stereo | Ambient, film, R&B, lo-fi, pop |
| Brass and stabs | A filter envelope with a fast swell, velocity to cutoff | Funk, disco, synth-pop, house |
| Supersaw chords and leads | Seven-saw unison, wide, reverb and delay | Trance, EDM, pop, J-pop, hardstyle |
| Plucks and arps | Short filter envelope, tempo-synced delay | House, trance, synthwave, game music |
| Keys and comping | Velocity to level and tone, short release | Synth-pop, city pop, gospel |
| Leads | Mono or legato, glide, vibrato on pressure, sync | Prog, synthwave, G-funk |
| Wavetable growls and morphs | Position and warp swept fast, distortion, filter FM | Dubstep, drum and bass, bass music |
| Evolving textures | Slow table scans, random LFOs, long reverbs | Ambient, sound design, scoring |

## 3. Algorithms

- **Band-limited classic waves.** Several good ones; each trades cost for alias rejection:
  - *PolyBLEP*: a two-sample polynomial correction around each step. Poly and Analog use it today (Analog places the step a sample late so any edge gets both halves).
  - *PolyBLIT*: impulses placed at fractional positions by a fractional-delay filter, then integrated; "the third-order B-spline produces minimum aliasing distortion", with no table and short impulse responses ([CCRMA overview](https://ccrma.stanford.edu/~juhan/vas.html); J. Nam, V. Välimäki, J. Abel, J. O. Smith, ["Efficient antialiasing oscillator algorithms using low-order fractional delay filters", IEEE TASLP 2010](https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf)).
  - *DPW* (differentiated polynomial waveforms): a polynomial of the trivial saw, differentiated; higher orders suppress more (named on the CCRMA page). Our reading: its gain correction grows as the pitch falls, so in f32 it needs care at low notes.
  - *BLIT by summation formulae* is alias-free but needs two sines and a division a sample, and parameter changes can step (CCRMA).
  - *Oversampling* with a decimation filter, the brute-force fallback, and the right tool for the nonlinear parts (the ladder at 2x, see [Bass](bass.md#3-algorithms)).
- **Wavetables.**
  - *Mip levels* (built): each frame analysed and kept at 17 bandwidths half an octave apart; a note reads the widest level whose top harmonic stays under Nyquist, allowing for unison and warp. Worst alias of a saw at C8 about 57 dB down ([Wavetable](../design/wavetable.md#tables)).
  - *Integrated wavetables*: the table integrated K times offline, read with an order-N Lagrange interpolator, then differentiated K times with FIR differentiators; the cost is the same at any pitch, N should match K (K = 4 with N = 1 adds new artefacts), and higher K is sensitive to word length (at 24 bits, K = 5 was noisier than K = 4) ([A. Franck, V. Välimäki, "Higher-order integrated wavetable synthesis", DAFx-12](https://www.dafx.de/paper-archive/2012/papers/dafx12_submission_69.pdf)). Our reading: in f32, K of 1 or 2 is the safe end; it could replace some mip levels for memory, not for quality.
  - *Frame morphing*: linear between neighbouring frames (built). Spectral morphing (blending harmonics, not samples) is the alternative; our reading is that it costs a resynthesis per position change and belongs on the control thread.
  - *Warps* (built): Bend, Sync, Mirror and FM.
- **The supersaw.** Seven free-running saws at the curve above, a centre-against-sides mix, and a key-tracked high-pass (KVR). Our reading: seven saws and a sub are eight lanes, two 4-wide NEON registers, which ARMv7 LLVM will not vectorise by itself (see the shared topics page); at 8 voices that is 64 band-limited saws a sample, so it has to be hand-vectorised or it will not fit.
- **Filters.** The zero-delay-feedback ladder and state-variable filter (Poly), the OTA four-pole with its own saturation (Analog), and Huovilainen's per-stage `tanh` ladder at 2x (see [Bass](bass.md#3-algorithms)). A comb, formant or phaser-style filter is a common extra on wavetable synths (our reading of the type; not sourced).
- **Voices.** Rotation across cards with per-card offsets (Analog), stealing the quietest releasing voice then the oldest held (Poly), unison stacks spread across the stereo field (both).
- **Folding Wavetable into Poly.** Accepted in the plan. Wavetable already shares Poly's parameter ids for the filter, envelopes, LFO and voice (30 to 36, 40 to 43, 50 to 53, 60 to 62), so one engine with an oscillator type per oscillator (classic, table, supersaw) keeps presets loading by id (our reading of the [Wavetable](../design/wavetable.md) page).

## 4. What to measure

Proposed targets for stage 4 (ours, in brackets, until the baseline and listening confirm them).

| Id | Measure | Proposed target | Why |
|---|---|---|---|
| P1 | Aliasing | Worst alias under [−60] dB for every oscillator type at C7, under [−55] dB at C8, and during a [2]-octave-per-second sweep | No fizz |
| P2 | Supersaw detune | Each saw within [2] cents of the curve at every knob setting; at least [3] distinct beat rates in the envelope spectrum | Many beats, not one |
| P3 | Supersaw phase | Two strikes of one note differ (correlation of the first 50 ms under [0.9]) | Free-running sound |
| P4 | Level against unison and chord size | Unison 1 to 8 within [3] dB; an 8-note chord of every preset under 0 dBFS | No jumps, no clips |
| P5 | Filter tracking | Cutoff within [0.1] octave of the label at zero resonance; self-oscillation within [10] cents over [3] octaves at full key track | Honest knobs; a filter that plays in tune |
| P6 | Envelopes | Times within [10] % of the label; a retrigger with no step above [−60] dB | Playable, click-free |
| P7 | Table morph | A full position sweep with no step above [−60] dB; position, warp and frame changes zipper-free | Smooth movement |
| P8 | Stereo | Correlation [0.3 to 0.7] with spread; mono fold-down loss under [3] dB | Width that survives mono |
| P9 | Velocity | Level and cutoff follow the preset's curve within [2] dB or [0.1] octave | Even dynamics |
| P10 | Drift (Analog, and Poly's Age since 19 Nov) | Per-card spread as labelled at each Age; drift rate under [10] cents over [30] minutes | Alive, not out of tune |
| P11 | Cost | 8 voices of a two-oscillator, unison-3 patch under [50] % of the synth budget; 8 voices of supersaw under [100] % | Fit the MPC |
| P12 | Play routes | A route reaches as far as its depth says, in its target's unit: the mod wheel at the top on Pitch at 100 % within [1] cent of an octave up; the default route (the wheel on Vibrato at 50 %) within [5] % of half a semitone each way | Knobs that mean what they say |

## 5. Knobs and defaults

| Section | Controls players expect | Proposed defaults |
|---|---|---|
| Macros | Four to six per preset, named by the preset (Hype's model), on the first page | Set per preset |
| Oscillators (two) | Type (classic, table, supersaw), Wave or Table, Position, Warp and amount, Octave (32' to 2'), Semi, Fine, Pulse width, Sync, Level | Saw, 8', centred |
| Sub and noise | Sub shape (TubeSynth: Triangle, Saw, Square, Pulse) and level, Noise level and colour | Off |
| Unison | Voices (1 to 8), Detune, Spread, Mix (centre against sides), Supersaw curve | 1; at 7, the supersaw curve |
| Filter | Type, Cutoff, Resonance, Drive, Key track, Envelope amount, Velocity | LP 24, open, 50 % key track |
| Envelopes | Filter, Amp and a third (TubeSynth), 1 ms to 100 s, velocity | Amp A 2 ms, R 300 ms |
| LFOs (two) | Shape, Rate or sync, Delay, Fade, Key sync, destinations | Triangle, 5 Hz, free |
| Mod matrix | Source, Destination, Amount (pressure, velocity, key, envelopes, LFOs) | Pressure to vibrato |
| Voice | Voices, Mono, Legato or Retrigger, Glide, Bend range, Drift (Analog's Age) | 8 voices, poly |
| Output and FX | Drive or tube shape, Width, Chorus, Delay, Reverb saved with the preset | Off unless the preset uses them |

## 6. The presets players expect

A first canon of 48 across Poly and its oscillator types (our reading): warm pad, string pad, glass pad, dark pad, evolving pad, vowel pad, noise pad; poly brass, synth horns, brass stab; supersaw chord, supersaw lead, hoover-style detuned lead, trance pluck; house chord stab, organ-ish stab; square keys, soft keys, bell keys; pluck arp, sequence bass-and-lead split, sync lead, square lead, unison lead, portamento lead; vowel lead, morph pad, sync sweep, digital bell, harmonic pluck, growl bass, reese-style table bass, formant wobble; sweeps and risers, noise sweep, impact; and analog-card versions of the pads and brass (worn cards, chorus strings, poly brass). Each pair of A and B variants differs by its macros, not by a new patch.

## 7. Playing it on the MPC

- **Macros first.** The four Q-Links take the preset's first four macros, so every preset is playable without a menu; the full panel stays a page away.
- **Pressure** to vibrato, cutoff, table position or warp; the shared pressure shaper is built.
- **Chord pads and the arpeggiator** for stabs, pads and arps.
- **Movement you can see.** The table and the position playhead, the filter curve and the envelope drawn from the values, as FM's pages already do. A touch XY control for cutoff and resonance or position and warp would play like an instrument; it is a layout question for the [layout spec](../design/layout-spec.md), not ours to add.

## 8. What goes wrong

- Fizz on high notes, sync and fast sweeps.
- A supersaw that sounds like one chorus: evenly spaced detune, or phases reset at every note.
- Volume that jumps with unison, chord size or drive.
- Pads that don't move, and presets that differ only in the oscillator wave.
- A panel nobody plays: dozens of parameters and no macros.
- Clicks when a voice is stolen or retriggered.
- Cost that multiplies (voices × unison × oscillators) past the budget.

## What MPFree has today

- **Poly** (9 presets, 1 to 16 voices, 8 by default): PolyBLEP saw and pulse, triangle, sine, sub, noise, up to 4 unison stacks on both oscillators (oscillator 2 detuned the other way at 0.7 of the amount, so 8 saws at most, evenly spaced, random phases at note-on when unison is above 1), the ladder and state-variable filters, drive and distortion that keep level. Its Supersaw preset is this, not the seven-saw curve.
- **Analog** (11 presets): voice cards with drift and Age, poly-mod, the OTA four-pole, a bucket-brigade chorus.
- **Wavetable** (9 presets): 10 formula-made factory tables, WAV and sample import, four warps, unison to 8, Poly's filter, pressure to position and warp.
- **Missing:** macros, a supersaw curve and mix, a second LFO and a mod matrix in Poly, effects saved in presets, cost measured on the MPC for Wavetable.

## Sources

- Virtual analog oscillators (BLIT, PolyBLIT, PolyBLEP, DPW), CCRMA: https://ccrma.stanford.edu/~juhan/vas.html
- J. Nam, V. Välimäki, J. S. Abel, J. O. Smith, "Efficient antialiasing oscillator algorithms using low-order fractional delay filters", IEEE TASLP 18(4), 2010: https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf
- A. Franck, V. Välimäki, "Higher-order integrated wavetable synthesis", DAFx-12: https://www.dafx.de/paper-archive/2012/papers/dafx12_submission_69.pdf
- KVR forum, "Roland Supersaw: any idea how the original was done?": https://www.kvraudio.com/forum/viewtopic.php?p=4644620
- atosynth, "Super saw analysis ideas" (on A. Szabo's thesis): https://atosynth.blogspot.com/2021/07/super-saw-analysis-ideas.html
- MPC Standalone OS 3.6 user guide, TubeSynth and Hype (project files)
- [Vintage analog polysynths](vintage-poly.md), [Bass](bass.md) and their sources
