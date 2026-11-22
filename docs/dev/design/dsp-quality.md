<!--
covers: rust/mpfree-app/src/rate_probe.rs rust/mpfree-synth/src/table_osc.rs rust/mpfree-synth/src/analog.rs rust/mpfree-synth/src/squelch.rs rust/mpfree-synth/src/dsp.rs rust/mpfree-sampler/src/engine.rs rust/mpfree-sampler/src/bandlimit.rs rust/mpfree-fx/src/oversample.rs rust/mpfree-fx/src/adaa.rs rust/mpfree-fx/src/resample.rs rust/mpfree-fxlab/src/synth.rs rust/mpfree-fxlab/src/sampler.rs
checked: 2026-11-22 0df0743b
-->
# DSP quality: fizz, filters near the top and the sample rate

**Status:** plan approved by the owner on 2026-11-22; phase 0 (measures, the rate probe, reference renders, the full-project check) is being built; the rate probe's code is written and has not run on the MPC. **No sound has changed yet.** Design doc with both outside reviews and the answers to them: "DSP quality" (https://claude.ai/artifact/5jsLgS5ZaRoHzpvVbEUgZ5). Related pages: [FX quality](fx-quality.md), [Synth quality](synth-quality.md), [Drive and Character](fx-drive.md), [Synth library: architecture and roadmap](synth-architecture.md), [Disk streaming](disk-streaming.md), [Profiling](profiling.md).

## Why

Rates above 48 kHz add nothing a listener hears; what they add is room for processing. A filter near the top of the band and a curve that makes overtones both need room above 20 kHz, and the engine has little: it runs at a fixed 44.1 kHz in 128-frame periods, so only about 2 kHz sits between the top of hearing and the Nyquist frequency (22.05 kHz). Overtones made above it fold back down as inharmonic fizz, and a filter designed with the bilinear transform squeezes its response as its cutoff nears it.

The answer here is not a higher engine rate. Every engine would pay for it on every sample, and the gaps are fixed more cheaply where they live: band-limited tables and steps where a wave is made, antiderivative antialiasing (ADAA) where a curve bends it, oversampling only round the stage that needs room, and filters solved so they stay true near the top. The engine stays at 44.1 kHz in f32.

**Fizz is a defect only where it was not chosen.** Lo-fi, Crush, Vintage pads, FM feedback at 7 and pad Drive at 24 dB fizz on purpose; their rows say so and they get no target.

## Where MPFree stands

From the ratchet files on main (`rust/mpfree-fxlab/quality/synth.tsv`, `sampler.tsv`, 0df0743b). The FX are mostly clean; the synths and the sampler's small pitch changes carry the fizz that is left.

| Where | Worst fold-back at C5 / C6 | Why |
|---|---|---|
| Poly saw, pulse, tables | -78 / -84 dB | band-limited tables (the voice core) |
| Analog saw | -54 / -48 dB | polyBLEP at the host rate; never moved to the tables |
| Poly table and Wavetable with sync 50 | -45 / -39 dB | the jump at each sync restart is not smoothed (`table_osc.rs`, `Warp::Sync`) |
| Squelch square | -48 / -39 dB | the shaper that turns the saw into a square has no ADAA (it already runs at 2x) |
| Poly drive 24 dB / distortion 24 dB | -50 / -45 and -45 / -40 dB | the curve runs per voice at 1x |
| Sampler, a semitone up, 9 kHz out | spur -31.5 dB | the Hermite read for small pitch changes |
| Sampler, a fifth down, 9 kHz out | spur -43.6 dB, 1.1 dB quiet | the same read pitched down |
| Sampler, an octave down, 9 kHz out (Y5) | spur -7.7 dB, 3.0 dB quiet | the same read pitched down: its images fold into the band |
| Sampler, a fifth down, 13.5 kHz out (Y5) | spur -3.4 dB, 4.6 dB quiet | the same |

Nobody has measured the synth filters above 4 kHz, their self-oscillation pitch, or what each engine does with block sizes other than 128. Phase 0 adds those measures.

The ratchets guard an approved baseline within tolerances, not a promise that numbers only improve: a few rows were blessed worse on purpose, each with its reason in its pull request. Step 0.1 reruns them all.

### The rerun (step 0.1)

Every ratchet was run again at main e5518efc with this page's new rows, its numbers written fresh and compared with the baselines: quality (1,594 rows, release), synth (5,323, release), sampler (74, debug) and grain (115, debug). This run was on an x86-64 cloud machine; the runners run the same four ratchets on every pull request in the merge train. **Every row is within its tolerance.** All but 29 rows read exactly as blessed. The 29 that moved:

| Rows | Baseline | Fresh | Why |
|---|---|---|---|
| EQ `drawn_vs_heard_db`, and its HQ 2x row | 0.0266 and 0.0043 dB | 0.0042 and 0.0014 dB | fxlab's response measure changed after the bless ([FX quality](fx-quality.md), Measured differently, already gives 0.004) |
| Grain G13, the LFO on position and on spray (8 rows) | swing -0.26 to +1.25 dB further, level -0.23 to +0.03 LU | swing -0.20 to +0.84 dB, level -0.24 to +0.05 LU | two grain changes after the bless (each grain keeps its window for its whole life; the edges of a sound) move a random cloud's draws. The largest moves are the pad's swing with position swept, 0.23 dB more (its tolerance is 0.5), and pink's with spray swept, 0.41 dB less. |
| Crush's off-grid rows (16) and three synth rows | | the same to 0.002 dB | no code under them changed since the bless; most likely the machine's maths library, as the full-project check allows for |

The ratchet files keep the older baselines, which are looser than the fresh numbers for the EQ and the grain rows; blessing them would tighten the ratchet and change no sound.

## Rules for DSP work

Every new synth or effect follows these, and each step below is held to them. Over all of them sits one principle (from the second outside review):

> Every DSP-quality improvement must preserve the intended musical behavior, demonstrably reduce a relevant artifact, and remain reliable under the complete device workload. Improvements in isolated numerical metrics are not sufficient grounds for replacing an existing algorithm.

1. **The native rate unless a higher one is measured to help.** Borrow room stage by stage, never for the whole engine.
2. **Stop fizz where it starts:** band-limited tables, band-limited steps, ADAA, then oversampling.
3. **Reuse what exists:** `oversample.rs`, `adaa.rs`, `resample.rs`, `bandlimit.rs`, `mpfree-simd`. A new primitive only when those measure unsuitable.
4. **A quality choice on screen only where it is audible and the cost differs a lot.** Never a switch driven by CPU load.
5. **Heavy work off the audio thread;** nothing there allocates, locks or blocks.
6. **Cost is part of done:** every engine brings its ARMv7 count, its cost on the critical path of the core that runs it and its effect on the full-project renders, fits its first limit, and has device numbers before it is on by default.
7. **Fizz is chosen, never an accident:** fizz on purpose is documented and its rows say so.
8. **A fix replaces the old sound** (the owner's early-development rule): a blind, level-matched A/B, then the ratchet blessed and the change explained in the PR. While the A/B runs, the new path sits behind a switch; the switch goes once the owner picks. No compatibility modes.
9. **A render sounds like what you heard,** unless an HQ render is chosen on purpose.
10. **Every new engine brings its rows:** fizz rows (A1) and a reference comparison (Y8) for anything that makes overtones, filter rows (Y1, Y2) for any new filter, a finite-output row (Y4) for everything, in its own PR.
11. **The MPC has the last word:** workstation and ARMv7 counts guide, device numbers decide.

An accepted regression's PR says which number moved, by how much, what improved, whether it is audible and whether the row still meets its target, as the FX rounds' "Blessed worse, and why" notes do.

## The plan

Each step is its own draft PR to the merge desk. Each sound change is judged three ways: its numbers (the fizz rows and the reference, Y8), a blind level-matched A/B for the owner, and its cost. Phase 0 changes no sound. Phase 1's steps can run side by side once 0.2, 0.5 and 0.6 land; 2.3A can start any time; phase 3 waits for disk streaming's native-rate step.

| Step | What changes | Where | Judged by |
|---|---|---|---|
| **0.1** Rerun | Every quality report at current main, on the runner VM, into one baseline table marking fresh numbers, ratchet baselines and older doc figures | fxlab `quality`, `synth`, sampler | Every row within its tolerance; any drift explained |
| **0.2** New measures | Y1a to c, Y2a and b, Y3, Y4, Y5, Y6 and Y7 (below) | `mpfree-fxlab` (`synth.rs`, `sampler.rs`, a new `filters` module) | They report; Y4 and Y7's bit-exact class are gates |
| **0.3** Rate probe | Asks the driver which rates, formats and channel counts it allows, in `inventory` and in the pad player before its usual setup | `mpfree-app/src/main.rs`, `alsa.rs` | [The rate probe](#the-rate-probe); a device round with the owner's go |
| **0.4** Device timing | Bench cases for every oversampled effect and synth voice, timed alone, on each core's critical path (sound cores off and two) and inside a full project | `mpfree-bench`, `host_bench_mpc.py --afford` | Worst-block share against the first limits |
| **0.5** Reference renders | Y8: each engine rendered at 8x and brought down with a long linear-phase filter; the 8x reference checked against 16x before it is trusted | `mpfree-fxlab` (`synth --rate`) | 8x and 16x agree before any Y8 target is set |
| **0.6** Full-project check | Fixed test projects (dense synths, a distortion chain, granular drums, long stems, a bass into the limiter, fast automation, then all at once) rendered offline through the whole chain and compared with approved renders on every change, plus whole-song instruction counts. Device deadlines are checked once per phase, not per PR | `mpfree-fxlab`, `mpfree-bench`, CI | Render matches, or the PR explains the change; counts within the 5 % gate |
| **1.1** Sync | A band-limited step at each sync restart: its height read from the playing table level, at the restart's fraction of a sample. PolyBLEP first, a short minBLEP table if needed. Tests with restarts between samples, a moving table position and fast ratio changes | `mpfree-synth/src/table_osc.rs` (Poly's Table wave and Wavetable) | A1 sync rows -60 dB at C5 and C6; Y8 |
| **1.2** Analog's waves | Two experiments first. A: plain saw and pulse. B: with sync, poly-mod and pulse-width modulation. Each compares today's polyBLEP, a 4- to 8-point band-limited step and the voice core's tables. Then improve the step, move to tables, or mix the two; if the numbers don't settle it, the owner picks by ear | `analog.rs`, `osc.rs` | A1 Analog rows -70 dB at C5 and C6; Y8 harmonic loss; cost; blind A/B |
| **1.3** Squelch's square | First-order ADAA on the square's shaper with the exact antiderivative of its curve (below). Wave blends saw and square, so the saw gets the same half-sample delay. Compared with band-limiting the saw before the shaper | `squelch.rs` | A1 -60 dB at C5 and C6 |
| **1.4** Sampler read | A cleaner read for small pitch changes and pitch-downs. Candidates: 6- and 8-point windowed sinc from one shared table, an optimal 4-point polynomial. Tests at one and two semitones each way, a fifth and an octave down; the read's own frequency response; switching reads mid-note without a click. The plain copy at pitch and Vintage (`Pad::vintage`) stay | `mpfree-sampler/src/engine.rs`, `bandlimit.rs`, with the sampler refactor | S1 and Y5 -50 dB on every row, level within 1 dB |
| **1.5** Levels and DC | The presets still failing the level and DC rows, fixed in one pass, keeping each preset's intended loudness and tone | factory presets | L2, L3 met; changed presets on a listening page |
| **2.1** Poly's drive and distortion | Per voice, as now, one algorithm per patch setting, the same for every note. Candidates: second-order ADAA at 1x; 2x round the shaper only with first-order ADAA (Drive's Eco half-bands) | `poly.rs`, `dsp.rs`, `mpfree-fx/src/adaa.rs` | A1 drive and distortion -60 dB at C5 and C6; Y8 |
| **2.2** Synth filters | Fix only what Y1 and Y2 measure as wrong; a filter's character is not a defect. Suspects: Analog's ladder taking its saturation from the previous sample; the 0.45 x rate cutoff cap in `FilterCoefs::new` | `analog.rs`, `dsp.rs`, others as found | Y1a to c, Y2a and b; blind A/B |
| **2.3A** FX profile | Where the oversampled effects' time goes: half-bands against everything else | `mpfree-bench`, device timing | A report |
| **2.3B** Half-band prototype | One SIMD kernel for the IIR half-bands, bit-exact with today's loop, kept only if the whole effect gets faster on the device | `mpfree-simd`, `mpfree-fx/src/oversample.rs` | Whole-effect device time; every ratchet row and golden file unchanged |
| **2.3C** Iron and Soften | Back at the base rate, if Y8 and the owner's ears agree | `character.rs` | Y8; blind A/B |
| **3.1** Keep the original | A loaded file at another rate keeps pointing at its original, never only at the 44.1 kHz copy, and no sound's decoded audio is held twice | sampler load path, with disk streaming | A test |
| **3.2** High-rate playback | Automatic, no setting: at or above its own pitch a voice plays the prepared 44.1 kHz copy; pitched down, it reads the original through the band-limited read when the memory or streaming budget admits it, otherwise the copy, and that fallback is counted, logged and shown | `mpfree-sampler`, `mpfree-stream` | Y6; fallbacks shown |
| **4.1** Export conversions | `resample.rs`'s claims checked ratio by ratio (44.1 to 48, 48 to 44.1, 96 and 192 to 44.1, 44.1 to 96): passband, transition band, stopband, phase, the start and end of a file, and a render done in pieces matching one done whole | `mpfree-fx/src/resample.rs`, `bounce.rs` tests | Per ratio: flat within 0.01 dB to 20 kHz; 100 dB down wherever energy would fold below 20 kHz; transition band documented |
| **4.2** HQ render | Design only, later | | The owner's call |

### Squelch's square: the antiderivative

The square is `SQ_LEVEL * tanh_pade(SQ_GAIN * (saw - SQ_THRESHOLD))`, where `tanh_pade(x) = x(27 + x²)/(27 + 9x²)` with x clamped to ±3 (where it reaches ±1). Dividing out, `tanh_pade(x) = x/9 + (8/3)·x/(x² + 3)`, so with F(0) = 0:

- for |x| ≤ 3: F(x) = x²/18 + (4/3)·ln(1 + x²/3);
- for |x| > 3: F(x) = F(3) + (|x| − 3), with F(3) = 1/2 + (4/3)·ln 4 ≈ 2.348.

F is evaluated in 64-bit (the difference of two nearby values loses precision in f32), and when two inputs nearly match the step uses the curve at their midpoint.

## Targets and new measures

New measures join fxlab and its ratchets in steps 0.2 and 0.5. As today, a target not met is printed, not failed, except the rows marked as gates.

| Id | What is measured | Target |
|---|---|---|
| Y1a | Small signal: every synth and sampler filter fed -40 dBFS, cutoff at 5, 10 and 15 kHz, resonance 0, 0.5 and 0.9, against its own model's response. 18 to 20 kHz is characterised, not targeted | Within 1 dB below the cutoff |
| Y1b | Driven: the same at -20, -10 and -3 dBFS, where the filter is meant to saturate | Reported |
| Y1c | Modulated: cutoff and resonance swept fast, by envelope and at audio rate | Stays finite and bounded; stepping noise no more than 3 dB over a smooth sweep |
| Y2a | Resonance tracking, for filters where the cutoff names the resonant peak: the peak's pitch against the cutoff, 100 Hz to 10 kHz | Within 25 cents |
| Y2b | Self-oscillation, only for filters meant to sing: started by one impulse at full resonance, its pitch against the cutoff and its level over 10 s | Within 10 cents; level bounded |
| Y3 | Audio-rate modulation: Analog's poly-mod at full into pitch and cutoff; fast filter envelopes | Reported only (partly the sound) |
| Y4 | Every engine and effect on its own, fed silence, full-scale noise and tones, in layers: each control alone at its ends, default and inside; chosen pairs; seeded random sets; fast sweeps; known dangerous corners | **Gate:** output always finite, without leaning on the chain's protection |
| Y5 | Sampler tones at 3, 9 and 13.5 kHz out, one and two semitones up and down, a fifth down, an octave down (extends S1) | Spur under -50 dB; level within 1 dB |
| Y6 | A 96 kHz file's 30 kHz tone played an octave down | Original read (3.2): heard at 15 kHz within 1 dB. Copy fallback: under -50 dB, and reported as a fallback |
| Y7 | Each engine's output at block sizes 1, 37 and 128 against 128, with the same event times and start state. Four classes: bit-exact; within a stated tolerance; equal by ear only; block-dependent by design (the last two documented) | The bit-exact class is a **gate**; the rest reported |
| Y8 | Reference: the engine at 44.1 kHz against its own 8x render brought down with a long linear-phase filter. Fizz energy, error in the intended harmonics, spectral envelope, level; error over time only where phase is fixed | Set per step in 0.5, once 8x and 16x agree |

### First results (Y4 to Y8)

Built in `rust/mpfree-fxlab`: `src/robust.rs` and `tests/robust.rs` for Y4 and Y7, `src/reference.rs` and `tests/reference.rs` for Y8 (all release only, in one CI step, about 50 s on four cores), and Y5 and Y6 in `src/sampler.rs` (the sampler ratchet, `quality/sampler.tsv`). Measured at e5518efc.

- **Y4: every subject stays finite.** 62 subjects (the 12 engines with their first preset's settings and no effects, the first drum kit, every registered effect), 20,208 renders. Each render is 0.6 s. An instrument plays notes 0 and 127 at velocities 1 and 127, a chord, bends to both ends, full pressure, the mod wheel and breath. An effect gets silence, full-scale white noise, a full-scale sine sweep and silence again. The layers are: the defaults; each control alone at its bottom, top and middle; all at the bottom, all at the top, alternating; 16 seeded random settings; every pair of controls in one group at their tops; each control swept from bottom to top and back in 0.3 s. No render held a NaN or an infinity. The loudest finite peaks (shown, not gated) come from gains at their tops: the channel strip +49 dBFS, multiband +49, Drums +34, Analog +33.
- **Y7: almost everything is bit-exact for any block size.** Notes and a seeded parameter move every 700 to 4,700 frames, rendered in blocks of 1 and 37 frames, against 128. Every engine, the drums and 46 of the 49 effects are bit-exact, and `tests/robust.rs` now holds all 59 to it. Not bit-exact: the chorus (-2.4 dB against the peak) and the phaser (-16 dB), whose LFO takes a new rate and re-syncs only at the start of a block (a defect to fix, not a design); the rotary differs only by rounding (below -400 dB).
- **Y5: the sampler's read fails pitch-downs near the top.** At 3 kHz out, everything meets -50 dB except an octave down (-44.5). At 9 kHz out: a semitone and two semitones down -28.9 and -26.3, two up -34.8, an octave down -7.7 dB with the tone 3.0 dB quiet. At 13.5 kHz out: -13.5 to -19.0 dB at one and two semitones each way, and a fifth down -3.4 dB with the tone 4.6 dB quiet. These are the read's images folding back into the band. Step 1.4's target covers every row.
- **Y6: the engine reads a 96 kHz sound at its own rate.** Its 30 kHz tone played an octave down is heard at -4.1 dB. The lab hands the engine a 96 kHz sound directly; whether loading a file keeps it at 96 kHz is what step 3.1 checks. The 4 dB loss is the read's own response that high, which step 1.4 also covers.
- **Y8: the 8x reference has converged in 55 of the 96 cases.** Each A1 case (31 engines and settings at C5, C6 and C7) is played as A1 plays it, at 44.1 kHz and at 8 and 16 times the rate, after as long a settle in seconds. The high renders come down to 44.1 kHz through a Kaiser linear-phase filter (flat within 0.001 dB to 20 kHz, more than 115 dB down from 24.1 kHz; 671 taps at 8x). The 8x reference counts as converged when its harmonics are within 1 dB of the 16x one's and its fizz within 3 dB, or both under the -80 dB floor of the measure. `tests/reference.rs` holds the 55 to that; `fxlab reference --sixteen` prints the table. Y8's fizz is all the energy off the harmonics, so it reads higher than A1's loudest single product (Poly's saw at C5: -67.7 dB against a converged reference at -86.2).
  - **Converged:** the clean oscillators of Poly, Squelch, Bass and Wavetable, Poly's table, Wind, Choir, Drawbar's combo flute, and Poly's distortion and Squelch's and Bass's drive.
  - **Not converged:** Poly's drive and Analog's drive 100 still fold at 8x (the drive's fizz 6 dB lower at 16x; Analog's harmonics 2.5 to 4.2 dB apart); sync (harmonics 1.3 to 6 dB apart); Squelch's square; every FM case (feedback 7 is noise by design); Analog's saw at C6 and C7 (fizz 3.7 dB apart, just above the floor). Strings and Drawbar's combo reed are a different case: their harmonics agree within 0.3 dB, but their energy off the harmonics grows by about 8 dB from 8x to 16x, so it is not folding but something that depends on the rate. Both play the divide-down bank (`mpfree-synth/src/bank.rs`), the first place to look. Steps 1.1 and 2.1 set their Y8 targets against the 16x render, or after the cause is found.
  - **What the converged references show beyond fizz:** below 16 kHz, Poly's saw, pulse and tables are 4 to 6.5 dB brighter at 44.1 kHz than their references. The plain patch's filter is set to 20 kHz: at 8x it takes 12 dB off at 20 kHz, as an analog four-pole would, but at 44.1 kHz, that close to Nyquist, the same setting lets nearly everything through below 16 kHz (step 2.2's question: which one is intended). Strings and the combo reed go the other way, 14 to 17 dB darker near 15 kHz, and Choir 6 to 11 dB. Wind is 1.4 to 3.1 dB quieter at 44.1 kHz: the bore models change with the rate. From 16 to 20 kHz, Poly's oscillators keep no harmonics above about 16.7 kHz, where their references keep them all; that top-end loss is what step 1.2's "harmonic loss" is measured by.

**Fizz targets by kind:**

| Kind | Target | Today |
|---|---|---|
| Clean oscillators: saw, pulse, tables | -70 dB at C5 and C6, C7 reported | Poly -78 and -84; Analog -54 and -48 |
| Hard sync, wavetable warps, FM feedback up to 5 | -60 dB at C5 and C6 | Sync -45 and -39 |
| Synth drive and distortion, per voice | -60 dB at C5 and C6 | Poly drive -50 and -45, distortion -45 and -40 |
| Sampler Drive at 12 dB; FX smooth curves | -65 dB with a 5 kHz sine (S6, G1) | Met |
| FX hard curves at Eco 2x | -50 dB with a 5 kHz sine | Fuzz and Rectify just short; HQ 4x meets them |
| Fizz on purpose | Documented, no target | |

The gates that already hold stay: nothing allocates on the audio thread (the counting allocator), the instruction gate fails any bench case 5 % dearer, and every ratchet row stays within its tolerance. Step 0.6 adds one: the full-project renders match their approved versions, or the PR says what changed and why.

## Cost

Every step has two requirements: a quality one (its targets) and a resource one, met under approved workloads. Cost is measured three ways: alone (the engine by itself), on the critical path (the core that has to finish it, with sound cores off and with `--sound-cores two`), and in a full project (step 0.6's renders, and on the device once per phase). A period is 2.9 ms, about 4.7 million cycles of one core. The audio thread's worst block stays under 60 %; synths get 20 % (580 µs), FX 10 % (290 µs), the sampler 25 % (726 µs) ([Profiling](profiling.md)).

The limits below are **first limits**, set before building. When a better candidate needs more, the trade goes to the owner with its numbers; it is neither dropped nor let through quietly. A saving counts only on the core where it lands.

| Step | First limit | Why that limit |
|---|---|---|
| 1.1 Sync | 5 % more per voice while sync is on; nothing while off | One correction per restart, not per sample |
| 1.2 Analog's waves | 10 % more than today's Analog at most | Each candidate's cost is measured in the experiments before a choice |
| 1.3 Squelch's square | 10 % more on its one voice | A mono synth; ADAA is a few operations a sample |
| 1.4 Sampler read | The sampler's bench cases 15 % dearer at most; plain copies at pitch unchanged | 64 voices share it and only pitched voices pay; with sound cores on, the sampler's own core is the one that must fit |
| 2.1 Poly's drive | 1.3 times the voice while drive or distortion is on; nothing while off | Most Poly sounds have neither on |
| 2.2 Synth filters | Fix the solve first: a 2x voice costs about three times an Analog voice today, too dear for 8 to 16 voices | |
| 2.3 FX cost | Saves, never spends: kept only if the whole effect is faster on the device | It pays for nothing on another core |

NEON kernels stay bit-exact with their plain loops (no fused multiply-add on ARMv7), as the supersaw, the Choir's formants and the banks already are, so every ratchet and golden file stays valid. A tolerance only if a kernel truly can't be.

## The rate probe

The supported sample rates of the MPC One's converters are not known ([Audio runtime](../hardware/audio-runtime.md) records only 44.1 kHz). The codec is `inmusic,acva-audio-codec`, configured by pins with no control bus, behind the RK3288's I2S block. Step 0.3 asks the driver, read-only:

- For each of 44.1, 48, 88.2, 96, 176.4 and 192 kHz, on card 1's playback and capture devices (`pcmC1D0p`, `pcmC1D0c`), it asks whether the driver would accept that rate (the HW_REFINE question, `Pcm::refine` in `alsa.rs`), and prints the formats and channel counts it allows.
- It runs in `mpfree inventory` (step 4 of `host_app_mpc.py`, on a stick whose Akai app is not running), which opens each device itself. On a stick that starts the pad player at power-on the player holds the playback device and inventory sees it busy, so the pad player asks the same questions of the playback device it has just opened and logs the answers before it sets up its usual stream (`open_acva` in `main.rs`). The code is `rust/mpfree-app/src/rate_probe.rs`; its tests check each rule below that a workstation can check.

**Rules the probe's code keeps:**

- It only asks (HW_REFINE). It never sets a format (HW_PARAMS), prepares, starts, reads or writes audio.
- It opens each device non-blocking, and a busy device is reported once, never retried.
- Every device it opens is closed again on every path, failures included.
- Nothing it finds can stop inventory finishing; a refusal is printed and inventory carries on.
- It reports formats and channel counts beside the rates.

A yes from the driver does not prove the converters run at that rate; that needs a tone through the outputs and back, a separate step that changes the stream for one session and gets its own go. What a yes would mean:

| The driver allows | What it means | What to do |
|---|---|---|
| Only 44.1 kHz | Nothing changes | Record it in [Audio runtime](../hardware/audio-runtime.md) |
| 48 kHz as well | Twice the room above 20 kHz for about 9 % more CPU in every engine | Not by default; revisit only if phase 2 finds problems 48 kHz would fix cheaply |
| 88.2 or 96 kHz | The whole engine at that rate costs about twice | Not for playing; a record-only 96 kHz sampling mode would be its own design |

## Who builds what

The DSP quality thread builds phases 0 to 2 and step 4.1. Step 1.4 goes with the sampler refactor (it owns `engine.rs`), and phase 3 with disk streaming (its Native mode already reads files at their own rate). The probe, device timing and every A/B run on the owner's go.
