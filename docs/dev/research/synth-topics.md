# Synth library: topics every type shares

**Status:** research, 2026-11-18 (synth library stage 1). The common ground under the type pages: [Bass](bass.md), [Keys](keys.md), [String machines](string-machine.md), [Poly and wavetable](poly-synth.md), [FM](fm.md), [Organs, winds, choirs and tape](organ-wind-choir.md) and [Drums](drums.md). Plan: "Synth library: stock-take and research plan" (https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). Nothing here has run on the MPC.

Where a point comes from a source it is linked; where it is our own reading, arithmetic or a target we propose, it says so.

## 1. Voices and the CPU

- **The budget is a slice of one core, not a hardware limit.** Every synth track's instrument together gets 20 % of the worst block, 580 µs of a 2.9 ms period; the audio thread as a whole gets 60 %, and an engine is judged on its p99.9 block ([Profiling, budgets](../design/profiling.md#budgets-agreed-with-jay-2026-11-11)). The split was set on 11 Nov with everything on one core. Round 7's bench put 8 Poly voices at 7 to 8 % of a period on average and 17 to 18 % at p99, with the pad player running beside it, so the p99 is an upper bound ([round 7](../perf/mpc-one-round7.md)). Our arithmetic: on one core the slice holds about 8 Poly Init voices in the worst blocks and about 20 on average, for every synth track together. The worst block is 2 to 3 times the mean, so spikes cost as much room as steady work.
- **The other cores.** The MPC One has four Cortex-A17 cores; the test kernel keeps cores 2 and 3 for us, and in the default "Shared" mode the sound runs on cores 0 and 1 while 2 and 3 sit idle ([Concurrency](../design/concurrency.md#where-things-run-today)). Akai's app on the same hardware runs an "Audio Processing" thread and four "AudioWorker" threads ([Audio runtime](../hardware/audio-runtime.md)), which suggests it renders across the cores. The levers that raise the synths' room, best first (our reading, to be measured): a device A/B of the built Sound cores modes; synth tracks scheduled across cores 1 to 3 with a per-core budget; a voice split inside one heavy synth, as the sampler has; rendering sequenced tracks a few periods ahead on spare cores (the 14 Nov review's "biggest lever", [Profiling](../design/profiling.md#the-stability-and-capacity-review-2026-11-14)); flatter worst blocks (note starts, denormals, the CPU clock); NEON kernels; algorithms that are cheap by design; a higher line on an isolated audio core; and a bigger buffer as an option.
- **Voices per engine** (accepted in the plan): Poly, Analog and Wavetable 8 by default, up to 16; FM 16; electric piano 16; string machine and organ every key, because the cost is per oscillator bank, not per key; mono 1; drums 32 hits. Each default waits on device numbers ("new engines must come with device numbers before they are turned on by default", Profiling).
- **A second core.** Lane B on core 2 is built for the sampler, mixer and returns ([Concurrency](../design/concurrency.md)); the pad player's synth can run on core 1 as a test (`three`). Our reading: scheduling every synth track across the cores belongs in the stage 3 architecture.
- **NEON is not automatic.** LLVM does not auto-vectorise `f32` loops for ARMv7 NEON, and the explicit API is nightly-only today ([Profiling, NEON](../design/profiling.md#neon)). Our reading: engines should lay voices out as structure-of-arrays so a later NEON or fixed-point path is a drop-in; the supersaw (7 saws), the choir (5 formants) and the string machine's bank are the first candidates.
- **Load shedding** gives up the quietest voices when periods fall behind ([Profiling, load shedding](../design/profiling.md#load-shedding-giving-up-voices-nobody-hears)); it is a safety net, never a budget.
- **Cost flat with keys** is a target in its own right for divide-down engines (S5, O5).

## 2. A shared voice core

Every engine re-implements parts of the same voice: envelopes with exponential segments that retrigger from the current level, LFOs free or synced, glide, a note stack for mono and legato, voice stealing with a 3 ms fade in spare slots, pressure through the shared shaper, the drive-level correction. Our reading: one shared core (as `pressure.rs` and `dsp.rs` already are in part) makes behaviour consistent across engines, lets every engine gain features at once (note priority, a constant-rate glide, a second LFO, a mod matrix, macros) and keeps tests in one place. It is a stage 3 decision.

## 3. Anti-aliasing

Each type has its answer: PolyBLEP for classic waves, mip levels for tables, keyboard scaling for FM, 2x oversampling for nonlinear filters, band-limited squares for drum metal ([Poly and wavetable](poly-synth.md#3-algorithms), [FM](fm.md#3-algorithms), [Drums](drums.md#3-algorithms)). Proposed shared target: worst alias under [−60] dB at C7 for every preset, measured the same way in every engine (ours).

## 4. Analog character

Per-voice offsets and slow drift ([Vintage analog polysynths](vintage-poly.md#voices-and-why-no-two-are-alike)), per-hit variation in drums, tape wow and flutter, the bucket-brigade roll-off in choruses and ensembles. Our reading: one seeded "character" module with labelled amounts (Age, Variation, Wow) shared by engines, always off at zero and bit-exact there, as Poly's drive is.

## 5. Expression

- **Velocity.** Velocity should change tone, not just level, on every type that has a struck or plucked source. A square-law level curve over a per-preset dynamic range of [30 to 40] dB is a well-founded default ([R. Dannenberg, "The interpretation of MIDI velocity", ICMC 2006, as Csound's `ampmidid`](https://csound.com/manual/opcodes/ampmidid/)). Pads spread velocities differently from keys, so the pads' curve and each preset's range are tuned together ([Keys](keys.md#7-playing-it-on-the-mpc)).
- **Pressure** is per note on the MPC's pads and already reaches every engine through the shared shaper. It is the swell for strings, the breath for winds, brightness for FM, vibrato for leads.
- **Per-note expression** (`NodeEvent::Expression`: Slide, Tone, Release) carries MPE-style pitch, timbre and release velocity.
- **The gaps.** Instruments hear NoteOn, NoteOff, Pressure, Bend, Expression, Param and AllOff. MIDI controllers reach them only as learned parameters (MIDI Learn) or as Q-Links (MIDI Follow) (`rust/mpfree-app/src/midi_in.rs`). So there is no sustain pedal (CC 64), and no channel-wide breath (CC 2) or mod wheel (CC 1) routed as expression. A hold or sustain event, and a channel controller event, are stage 3 architecture items.

## 6. Effects belong to the preset

Players judge a preset with its effects: tines through a phaser, strings through an ensemble and reverb, drums through a transient shaper and compressor ([Keys](keys.md#1-what-makes-a-great-one), [String machines](string-machine.md#5-knobs-and-defaults), [Drums](drums.md#1-what-makes-a-great-one)). The `mpfree-fx` units exist after the FX quality rounds; presets can't carry them yet. Our reading: a preset's own insert chain (two or three units) and its send levels, saved with the preset and loaded with it, is the single change that most lifts the library.

## 7. Macros and morphing

Akai's Hype puts six macros on every preset (MPC 3.6 user guide); the Digitone's X/Y mix makes four operators sound like many ([FM](fm.md#1-what-makes-a-great-one)). Our reading: every engine gets four to six named macros per preset, each moving several parameters along curves the preset defines; the Q-Links take the first four. Morphing between two presets (A to B on one knob) is a natural extension once macros exist.

## 8. Preset craft

- **Level rules.** Each engine already has a level rule (for example, FM chords between −11 and −6 dBFS peak; Analog single notes −17 to −10 dBFS). Proposed: one rule for the whole library, every preset's held chord or phrase within [3] LU of short-term loudness of the others of its type (ours), so browsing doesn't jump.
- **Names and tags.** Names say what a preset is for ("Ballad Tines", "Rolling Hats"), tags give type, character and genre for the browser's search; Akai's preset names are never copied.
- **Variants.** An A and B of a preset differ by their macros, not by a new patch.
- **Count.** The type pages' canons add up to over 230 presets and 24 kits (our arithmetic: bass 32, keys 32, strings 24, poly and wavetable 48, FM 40, organs, winds, choirs and tape 32, plus 24 drum kits): the "hundreds" the owner asked for, made in waves.
- **The browser.** Try, Load and Save aren't wired in the app yet ([Presets, not yet](../design/presets.md#not-yet)); the library is only useful once they are.

## 9. Playing it like an instrument

- **Pads:** 16, velocity and per-note pressure, chord pads and scales, Note Repeat and the arpeggiator. **Q-Links:** four knobs a page. **Screen:** touch, drawn visuals of what the sound is doing (FM's algorithm, Drums' hit, the envelope as it runs). **MIDI in:** keyboards with a sustain pedal and, for some, MPE.
- Our reading of what makes it feel like an instrument rather than a program: the first page of every preset is playable (macros on the Q-Links); every knob moves the sound without a click or zipper; what you hear is drawn (a filter that moves, a table playhead, an ensemble's lines); pressure does something musical on every preset; and nothing needs a menu to start playing. Layout changes go through the [layout spec](../design/layout-spec.md) and its owner.

## 10. How we will judge quality

The FX quality method, applied to synths: the targets tables on each type page (proposed values in brackets until a baseline confirms them), a measure-only baseline of today's engines, a quality ratchet on CI, the instruction gate, blind loudness-matched A/B listening pages as in the FX rounds, and device rounds for cost. Listening covers each engine and a sample of its presets.

## Sources

- [Profiling](../design/profiling.md), [Concurrency](../design/concurrency.md), [Presets](../design/presets.md), [Wavetable](../design/wavetable.md) and [Synths](../design/synths.md) (MPFree design pages)
- R. Dannenberg, "The interpretation of MIDI velocity", ICMC 2006, as Csound's `ampmidid`: https://csound.com/manual/opcodes/ampmidid/
- MPC Standalone OS 3.6 user guide, Hype (project files)
- The type pages listed at the top and their sources
