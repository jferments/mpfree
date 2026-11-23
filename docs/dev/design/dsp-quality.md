<!--
covers: rust/mpfree-app/src/rate_probe.rs rust/mpfree-synth/src/table_osc.rs rust/mpfree-synth/src/analog.rs rust/mpfree-synth/src/squelch.rs rust/mpfree-synth/src/dsp.rs rust/mpfree-sampler/src/engine.rs rust/mpfree-sampler/src/bandlimit.rs rust/mpfree-fx/src/oversample.rs rust/mpfree-fx/src/adaa.rs rust/mpfree-fx/src/resample.rs rust/mpfree-synth/src/bass.rs rust/mpfree-fxlab/src/synth.rs rust/mpfree-fxlab/src/sampler.rs rust/mpfree-fxlab/src/robust.rs rust/mpfree-fxlab/src/reference.rs rust/mpfree-fxlab/src/filters.rs
checked: 2026-11-22 33571aaf
-->
# DSP quality: fizz, filters near the top and the sample rate

**Status:** plan approved by the owner on 2026-11-22; phase 0 (measures, the rate probe, reference renders, the full-project check) is being built; the rate probe's code is written and has not run on the MPC. Phase 1 has begun: step 1.1 changes the sync warp's sound on the table oscillator ([Step 1.1: sync](#step-11-sync)), and step 1.2 Analog's oscillators ([Step 1.2: Analog's waves](#step-12-analogs-waves), built, its blind A/B next); no other sound has changed yet. Design doc with both outside reviews and the answers to them: "DSP quality" (https://claude.ai/artifact/5jsLgS5ZaRoHzpvVbEUgZ5). Related pages: [FX quality](fx-quality.md), [Synth quality](synth-quality.md), [Drive and Character](fx-drive.md), [Synth library: architecture and roadmap](synth-architecture.md), [Disk streaming](disk-streaming.md), [Profiling](profiling.md).

## Why

Rates above 48 kHz add nothing a listener hears; what they add is room for processing. A filter near the top of the band and a curve that makes overtones both need room above 20 kHz, and the engine has little: it runs at a fixed 44.1 kHz in 128-frame periods, so only about 2 kHz sits between the top of hearing and the Nyquist frequency (22.05 kHz). Overtones made above it fold back down as inharmonic fizz, and a filter designed with the bilinear transform squeezes its response as its cutoff nears it.

The answer here is not a higher engine rate. Every engine would pay for it on every sample, and the gaps are fixed more cheaply where they live: band-limited tables and steps where a wave is made, antiderivative antialiasing (ADAA) where a curve bends it, oversampling only round the stage that needs room, and filters solved so they stay true near the top. The engine stays at 44.1 kHz in f32.

**Fizz is a defect only where it was not chosen.** Lo-fi, Crush, Vintage pads, FM feedback at 7 and pad Drive at 24 dB fizz on purpose; their rows say so and they get no target.

## Where MPFree stands

From the ratchet files on main (`rust/mpfree-fxlab/quality/synth.tsv`, `sampler.tsv`, 0df0743b). The FX are mostly clean; the synths and the sampler's small pitch changes carry the fizz that is left.

| Where | Worst fold-back at C5 / C6 | Why |
|---|---|---|
| Poly saw, pulse, tables | -78 / -84 dB | band-limited tables (the voice core) |
| Analog saw | -54 / -48 dB | polyBLEP at the host rate; never moved to the tables; -94 / -89 dB since step 1.2 |
| Poly table and Wavetable with sync 50 | -45 / -39 dB | the jump at each sync restart is not smoothed (`table_osc.rs`, `Warp::Sync`); -72 / -85 dB since step 1.1 |
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

### Step 1.1: sync

Built, and kept: the owner's blind, level-matched A/B (22 Nov; Poly's Table wave and Wavetable at sync 50 on a line, both Digital Bells, a fast warp sweep) was a tie in all five pairs, so the fix replaces the old sound on its numbers. The sync warp of the table oscillator (`table_osc.rs`, so Poly's Table wave and the Wavetable synth alike) restarts the read at each wrap of the running phase, and the read jumps there in its level and in every derivative. Each restart now gets a band-limited correction at its own fraction of a sample:

- **The jumps** (`Restart`). Once a control step: the read's level and first three derivatives at the ratio's fraction of a cycle (just before a restart) and at phase 0 (just after), from the playing level by central differences a table sample apart, on the frames crossfaded as the step has them, at the ratio and crossfade the step starts with and at those it ends with. In between, a straight line, exact at both ends. What the step before worked out is used again while it holds: the same frames, and the ratio within 0.00001 of where it was.
- **The residuals** (`Edges`). Each jump, times the read's step a sample to its power, scales a residual: the running integrals of a Blackman-Harris windowed sinc (cutoff 0.43 of the rate, 12 samples either side, its second moment taken out) less the plain step, ramp, parabola and cubic, tabled 128 points to a sample. A restart's time is known ahead, so the 12 samples before it are corrected as well as the 12 after. This is the plan's "short table if needed": a polyBLEP on the level alone left the A1 rows near -50 dB, and the three derivative terms take them under -69.
- **The level a sync read picks.** The correction holds to -60 dB only while the read's top harmonic stays under about a third of the rate, so a sync read picks its level for 1.5 times its fastest ratio (`SYNC_MARGIN`), a level or two lower than before. The smallest levels are now 128 samples, not 64: the linear read's images on a 64-sample level were at -54 dB. A table frame takes 51 KB instead of 49 ([Wavetable synth](wavetable.md)).
- **A moving ratio glides.** The ratio used to step once a control step, so a fast warp move made a zipper (-17 to -21 dB against an oversampled read). It now moves a frame at a time across the step (`Warp::Sync { ratio, glide }`), bit-exact at any block size.

| Measure | Before | After |
|---|---|---|
| A1 Poly table, sync 50, C5 / C6 / C7 | -45.0 / -39.0 / -32.8 dB | -71.6 / -84.5 / -69.6 dB |
| A1 Wavetable sync 50, C5 / C6 / C7 | -45.0 / -39.0 / -32.8 dB | -71.4 / -84.9 / -69.6 dB |
| A1 position 100 at C7 (both engines); Wavetable Init at C7 | -54.0; -71.5 dB | -66.8; -84.3 dB |
| Y8 sync 50 fizz, C5 / C6 / C7 (both engines) | -34.8 / -31.6 / -28.2 dB | -67.6 / -83.0 / -68.3 dB |
| Y8 sync 50 top (16 to 20 kHz against the 8x render), C5 / C6 / C7 | +6.7 / +12.7 / +14.9 dB | -9.8 / -1.5 / +12.8 dB |
| Restarts between samples, three pitches, amounts 0.2, 0.5, 0.93 | | all under -60 dB |
| A fast ratio sweep with a moving position, against an oversampled read | -17 to -21 dB | -46 to -52 dB |

No other ratchet row moved past its tolerance; the four sync bench cases have new golden fingerprints.

- **What it costs in sound: some of the top octave at C5.** With the lower level, the harmonics from 16 to 20 kHz are about 10 dB under the 8x render at C5, where before fizz folded there and made them 7 dB over it. Y8's sync references haven't converged (the 8x and 16x renders are 5 to 6 dB apart), so the figure is rough. The blind A/B heard no difference.
- **A finding, not fixed here:** what is left in the fast sweep is the level switching while the ratio moves; with the level held, the same sweep is under -55 dB. A crossfade between levels would be a step of its own.
- **Cost** (instruction gate, thousands of x86-64 instructions a block, against the build before). Sync on: Poly Table, 8 voices, sync 50, 334 to 327 (-2.1 %); Poly Sync Sweep 446 to 447 (+0.3 %); Poly Digital Bell 363 to 370 (+1.9 %); Wavetable Init, 8 voices, sync 50, 413 to 424 (+2.6 %); Wavetable Sync Sweep 576 to 553 (-3.9 %); **Wavetable Digital Bell 483 to 510 (+5.6 %), over the first limit of 5 %** (the owner accepted it on 22 Nov, to be checked again by the device timing, step 0.4): its envelope moves both the sync and the table position at every control step on a single layer, so the per-step work is spread over the fewest samples. Sync off: Poly's table sounds up to 0.8 % (per-step bookkeeping; the sample loop is as it was), Wavetable's sounds 2 to 6 % cheaper (its sample loop now has a version without sync). Part of Poly's saving is that a sync read now finds its table index from integers: on x86 that saves a costly float-to-index conversion, on the MPC's ARM it is one instruction either way. Device timing (step 0.4) is the real check.

### Step 1.2: Analog's waves

Built; the owner's blind, level-matched A/B is next. Analog's oscillators (`analog.rs`) were band-limited with a polyBLEP placed a sample late. They now use a 32-sample band-limited step, chosen by two experiments.

**The experiments.** A scratch harness (`analog-1.2/` in the project files, not committed) rendered one oscillator pair as Analog runs it, with no filter, at C5, C6 and C7, and compared each method with the same pair rendered at 8 times the rate and brought down. Experiment A was the plain saw, pulse (10 and 25 %), square, Saw+Pulse and triangle; B was sync (saw, pulse and triangle, and a saw five times the master), poly-mod into pitch and width, and both. The methods were today's polyBLEP; a four-point cubic B-spline step; an eight-point Kaiser-windowed sinc; windowed sincs of 16, 24, 32 and 48 points; a 32-point one with its cutoff at 0.48 of the rate; and the voice core's tables, with an eight-point step at a sync restart. The harness reads lower than the engine's A1 rows, which go through the filter.

| Method (harness, worst fold-back, C5 / C6 / C7) | Saw | Pulse 10 % | Sync, saw | Width poly-mod | Sync and pitch poly-mod | 16-20 kHz against 8x | Cost, saw / sync |
|---|---|---|---|---|---|---|---|
| polyBLEP (before) | -43 / -39 / -32 | -35 / -29 / -26 | -38 / -31 / -24 | -36 / -32 / -25 | -39 / -30 / -28 | -5.1 dB | 128 / 214 |
| 4-point B-spline | -53 / -49 / -43 | -45 / -39 / -37 | -48 / -42 / -35 | -38 / -33 / -26 | -50 / -40 / -37 | -10.2 dB | |
| 8-point sinc | -48 / -45 / -38 | -40 / -35 / -33 | -43 / -37 / -31 | -37 / -33 / -26 | -46 / -36 / -34 | -3.5 dB | |
| 16 / 24 points | -52 / -62 at C5 | -43 / -53 | -47 / -57 | -38 | -52 / -65 | -0.4 / -0.1 dB | |
| 32 points, cutoff 0.48 | -94 / -99 / -95 | -94 / -94 / -92 | -99 / -95 / -90 | -38 / -33 / -27 | -71 / -58 / -47 | -0.2 dB | 157 / 332 |
| the same, oscillator 2 heard within the sample | -94 / -99 / -95 | -94 / -94 / -92 | -99 / -95 / -90 | -94 / -99 / -95 | -88 / -76 / -64 | -0.2 dB | 157 / 438 |
| the voice core's tables | -77 / -84 / -90 | -68 / -74 / -80 | -42 / -36 / -26 | -38 / -33 / -27 | -48 / -44 / -41 | -78 dB | 199 / 360 |

The 8x render's own floor is -94 / -99 / -95 dB. Cost is the harness's x86 instructions a sample for the pair.

- **Short steps don't reach -70.** Four or eight points leave the plain saw at -48 to -53 dB at C5; 24 points reach -62, 32 points -75, and 32 points with the cutoff at 0.48 of the rate (rather than 0.5) reach the floor.
- **Tables are clean but cut the top and don't help sync.** The voice core's tables keep no harmonics above about 16.7 kHz (-78 dB from 16 to 20 kHz, against -0.2 for the 32-point step), a restart still needs its own step, and there is no triangle table with corners to restart.
- **Poly-mod needs oscillator 2 as it is within the sample.** Read once a sample, a moving width or pitch is a step every sample; on width poly-mod every method stays at -36 to -38 dB at C5. With oscillator 2's plain wave heard where it jumps, the 32-point step reaches the floor there too.
- **Pitch poly-mod at 100 can't be read this way:** it spreads the note's energy everywhere, the 8x render included. Counting only what is 6 dB or more over the 8x render, the 32-point step with oscillator 2 within the sample takes C5 from -37 to -57 dB and C6 from -22 to -31; C7 stays at -9.

**What was built.**

- **The step** (`Steps`): a Kaiser-windowed sinc (cutoff 0.48 of the rate, 16 samples either side, beta 7) run up to one, less the plain step, and its running integral less the plain ramp, for a jump in slope; tabled 256 rows a sample, read with a straight line between rows. Built once, when the first Analog is made.
- **Where each jump lands.** A wrap, a pulse edge and a sync restart are jumps in level; the triangle's corners and a restart are jumps in slope. Each is placed at its own fraction of a sample. A pulse edge is found against the width as it moves across the sample, so a fast pulse-width move doesn't make a step every sample.
- **Sounding late.** Each oscillator sounds 16 samples (0.36 ms) late, so the step's correction reaches the 16 samples before a jump as well as after it.
- **A control step at a time.** Each oscillator makes the step's samples in a window of its own, `TAPS` samples still taking corrections followed by the step's own (`Osc::run`, one copy for each wave). Oscillator 2 goes first. When oscillator 1 follows it (sync, or its poly-mod into pitch or width), oscillator 2 notes its plain wave where each frame starts, its width, and where it wrapped and where its pulse fell. Oscillator 1 then cuts each sample at those points, with a step for each jump in its own level and slope (`Osc::run_follow`). A frame where oscillator 2 doesn't jump, under sync alone, is a plain sample. The cutoff's poly-mod hears oscillator 2's band-limited output.

| Measure | Before | After |
|---|---|---|
| A1 Analog saw, C5 / C6 / C7 | -53.8 / -48.2 / -41.8 dB | -94.3 / -89.0 / -82.9 dB |
| A1 Analog saw, Drive 100, C5 / C6 / C7 | -53.2 / -47.7 / -41.3 dB | -59.7 / -54.1 / -47.9 dB |
| Y8 Analog saw fizz, C5 / C6 / C7 | -45.1 / -41.9 / -38.5 dB | -81.3 / -78.5 / -75.4 dB |
| Y8 Drive 100 fizz, C5 / C6 / C7 | -43.3 / -40.2 / -36.8 dB | -46.8 / -43.7 / -40.6 dB |
| Y8 top (16 to 20 kHz against the 8x render), saw, C5 / C6 / C7 | -2.3 / -3.1 / -2.1 dB | +2.1 / +1.4 / +2.3 dB |
| Y3 poly-mod into pitch 100, with sync, C4 / C6 | -37.3 / -30.1 dB | -74.9 / -58.4 dB |
| Y3 poly-mod into pitch and cutoff 100, with sync, C4 / C6 | -34.2 / -26.4 dB | -36.3 / -32.2 dB |
| Y3 poly-mod into the cutoff 100, C4 / C6 | -54.5 / -34.8 dB | -52.5 / -32.1 dB |

The target was -70 dB at C5 and C6 on the A1 Analog rows.

- **The Drive 100 rows** are now set by the filter's own saturation, which Drive pushes up to 3 octaves harder, at the host rate.
- **The top octave is back, and the filter shows.** In the harness the oscillators keep their harmonics from 16 to 20 kHz within 0.2 dB of the 8x render, where the polyBLEP lost 5 dB and 2.3 dB from 8 to 16 kHz. Through the engine the top is now 1.4 to 2.3 dB over the 8x render: the host-rate filter lets more through near the top than it does at 8x (Y1a, step 2.2). Y8's harmonics and envelope rows (5.2 and 4.4 dB, from 2.6 and 2.5) measure the same thing.
- **Poly-mod into the cutoff is 2 to 2.7 dB fizzier,** and so is its 8x render (4.4 and 3.9 dB). Oscillator 2 now reaches the cutoff with its top octave. Against its references the 44.1 kHz render is 1 to 2.5 dB closer than before.
- **The reference moved too.** With oscillators flat to 0.48 of whatever rate they run at, the 8x and 16x renders feed more above the band into the filter's saturation: Analog's saw at C5 has 8x fizz of -76.5 dB against 16x's -80.8 (were -81.8 and -85.1), so it no longer counts as converged and leaves `tests/reference.rs`'s list. The 44.1 kHz render (-81.3) is under both.
- **Cost** (instruction gate, thousands of x86-64 instructions a block, against the build before; limit 10 % more): Analog Init 269 to 243 (-9.6 %), Poly Brass 206 to 174 (-15.3 %), Warm Pad 323 to 291 (-10.2 %), Worn Cards on 16 voices 470 to 413 (-12.1 %). The polyBLEP ran every oscillator a sample at a time through the whole voice. Running each oscillator alone for a control step, with a copy for each wave, saves more than the longer step costs. Every factory preset was counted the same way (8-voice chords): 13 of 14 are 1 to 16 % cheaper, and Clang Bell, poly-mod from oscillator 2 on two triangles, is 2.9 % dearer. Device timing (step 0.4) is the real check.

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

### The filter measures (Y1a, Y1b, Y1c, Y2a, Y2b) and Y3

Built in `rust/mpfree-fxlab/src/filters.rs` and held by `tests/filters.rs` against `quality/filters.tsv` (release only, in the same CI step, about 7 s). Poly's filter (Wavetable runs the same one) is measured directly. Analog's, Squelch's and Bass's go through their engine's `FilterProbe`: the voice's own solve, coefficients and make-up, and for Squelch and Bass the inner rate and half-band. The probes share their helpers with the voices, and every preset renders bit-identically with them in. Each sampler filter is played on a pad and divided by the same pad with its filter off. Every filter is compared with the curve its page draws. Resonance here is a share of each panel's range.

- **Y1a: at 44.1 kHz the filters bunch up near the top.** Poly's, Analog's and the sampler's filters are trapezoidal at the host rate. With the cutoff at 5 kHz, every one is within 1 dB of its curve below the cutoff (the worst 0.81 dB), except the sampler's 4-pole notch (1.1 dB) and its 4-poles with resonance (below). At 10 kHz the four-pole ladders are 1.1 to 3.5 dB off and the high-passes 3.0 to 3.3 dB. At 15 kHz they are 2.9 to 8.6 dB and 8.4 to 9.2 dB off. Bass's ladder runs at twice the rate and stays within 0.8 dB up to 10 kHz (0.6 to 1.8 dB at 15 kHz). Squelch's also runs at twice the rate, and is within 0.8 dB up to 10 kHz at resonance 0 and 0.5. At resonance 0.9, though, its peak is 1.5, 7.8 and 16.5 dB from the drawn one at 5, 10 and 15 kHz. From 18 to 20 kHz (characterised, no target) every host-rate low- and band-pass falls well below its curve, because the trapezoidal form puts a zero at half the rate.
- **The sampler's 4-pole picture is wrong with resonance.** The engine sets the second stage to half the resonance, but the Play page's curve (`mpfree-sampler-ui`, `filter_db`) draws both stages at full resonance. At 5 kHz the 4-pole low-, high- and band-pass read 3.4 dB off at resonance 0.5 and 13 dB off at 0.9, where the 2-poles are within 0.8 dB. The 4-pole low- and high-pass peaks sit about 26 cents from the drawn ones at every cutoff up to 2 kHz. The picture needs fixing, not the sound.
- **Y1b: the filters that saturate keep their gain until the resonance is up.** At resonance 0, Analog's, Squelch's and Bass's filters move by 0.12 dB or less up to -3 dBFS, and the sampler's Ladder doesn't move. At resonance 0.9 near the cutoff they compress, as they are meant to. At 5 kHz and -3 dBFS: Analog's LP 24 by 11.6 dB and its two-pole by 8.5, Squelch's by 15.7, Bass's by 10.1 and the sampler's Ladder by 15.2.
- **Y2a: resonance peaks follow their curves.** Every filter whose cutoff names a peak is within 25 cents of its curve's peak from 100 Hz to 10 kHz, except two. Squelch is 35 cents flat at 10 kHz, and the sampler's 4-pole low- and high-pass are 26 cents off (the picture above). Poly's LP 24 is 15.7 cents sharp at 10 kHz and Analog's 10.7.
- **Y2b: Analog's and Bass's ladders sing flat.** Each gets one small impulse at full resonance and runs for 10 s. The sampler's Ladder sings within 0.1 cent of its cutoff at every pitch, at a steady level. Analog's LP 24 sings 181 cents flat at 110 Hz, 132 at 440 Hz, 48 at 1.76 kHz and 7 at 7.04 kHz. Bass's sings 190, 162, 90 and 20 cents flat. Both hold a steady level from the fifth second on (within 0.01 dB). At the level they sing at, each stage's saturation lowers its cutoff. The ladder also takes each stage's saturation from the previous sample (step 2.2's first suspect). Which of the two causes most of the drop is the first thing step 2.2 sorts out.
- **How Y1c moves a filter.** Each filter moves as its engine moves it, a control step (16 frames) at a time. Poly's coefficients (and Wavetable's) and the sampler's are worked out once a step and held. Analog's, Squelch's and Bass's cutoffs move in a straight line of the filter's gain across the step, through each probe's `glide`. The sampler's filter runs through a copy of what its voice does (`PadMirror`), held to a real pad by a test (the same to f32 rounding, with an envelope sweeping it). There are four moves. A fast filter envelope rises from 100 Hz to 12.8 kHz in 1.5 ms and falls back with a 5 ms time constant, 21 times a second. An LFO swings two octaves either side of 1 kHz at 5.4 Hz. The cutoff swings three octaves at 689 Hz, and the resonance from 0 to 1 at the same rate.
- **Y1c: every filter stays finite and bounded.** Each filter is fed noise at -6 dBFS and moved every frame and at its engine's step, at resonance 0, 0.9 and 1. Its peak is compared with the loudest the same noise gets with the settings held at 16 places along the move. No sample is infinite or NaN. The worst peak is 4.9 dB over the held one (Squelch, on the envelope's rise; target 6 dB).
- **Y1c: held coefficients zipper on a fast envelope.** Each filter is fed a steady 1 kHz tone at -20 dBFS and moved at its engine's step, then every frame. The two are compared octave band by octave band, where the output is within 60 dB of its loudest band. Each move repeats in a whole number of steps and the tone fits the window, so the spectra have no leakage, and a step's lag doesn't count, only the noise it adds. Analog, Squelch and Bass glide and stay within 2.7 dB of the smooth move on the envelope, and 0 dB on the LFO. Poly's and the sampler's held coefficients add 13 to 21 dB on the envelope, mostly in the 8 and 16 kHz bands, 32 to 46 dB under the loudest band (the 2-poles worst; Poly's LP 24 4.9 dB, the sampler's Ladder 5.2, Vowel 3.2 and Comb 1.0). On the LFO they come to at most 4.2 dB, the zipper about 60 dB under the tone. Moving the coefficients in a straight line across the step, as Analog does, is the measured fix; it belongs to the step that next works on those voices.
- **Y3: Analog's poly-mod folds; fast envelopes don't.** Built in `reference.rs` with Y8's measures, at C4 and C6, and printed by `tests/reference.rs` (reported only). Oscillator 2 sits exactly at the note and is heard only through the poly-mod. Into the cutoff (1 kHz, resonance 50), the 44.1 kHz render has 23 and 16 dB more fizz than its 8x reference. Into oscillator 1's pitch with sync (so the note repeats and its harmonics can be read), it has 19 dB more, and into both, 20 dB more. None of those references has converged yet: each 16x render's fizz is a further 4 to 6 dB down. Without sync, pitch poly-mod makes no repeating note, and these measures don't apply. A fast filter envelope on Poly, Analog, Squelch and Bass (struck every eighth of a second, sweeping up from 200 Hz, decaying in 20 ms or at Squelch's shortest, 200 ms) has the same fizz as its reference within 0.3 dB. So the envelopes add no folding, and what differs is the filters' own response.

### The full-project check (step 0.6)

Seven fixed projects in `rust/mpfree-bench/src/projects.rs`, each 16 beats at 120 BPM that loop: dense synths, a distortion chain, granular drums, long stems recorded at 48 kHz, a bass into the limiter, fast automation, and all six at once. Each plays through the whole chain as a song does: the synths in the track host into the mixer's tracks, the sampler's pads into the pad channels and on into their tracks, every insert, the sends, the returns, the master chain and Master safety. They play from fixed scores, so the time engine's own cost stays in its own bench cases. Sounds and stems are made in code, so the repository holds no audio.

- **The render check:** each project's 16 beats and 2 s more are fingerprinted per half second and channel (the level, and the octave bands from 63 Hz to 16 kHz) and compared with `golden/projects.txt`: within 0.1 dB of level and 0.25 dB a band, leaving out bands more than 60 dB under the loudest. That passes another libm but fails any change a listener could hear. The check also says whether each render is still bit-identical to the approved one; that is reported, not gated, since another platform's libm changes the bits. It runs in release, in its own CI step, in about 2 s. A PR that changes the sound blesses it and says why.
- **Instruction counts:** each project is also a bench case, so the instruction gate counts it per block (200 blocks after the 50-block warm-up, with every part playing) and fails a rise over 5 %. Today: all at once 4.2 M instructions a block, the distortion chain 1.6 M, dense synths 1.3 M, granular drums 0.74 M, fast automation 0.57 M, the bass into the limiter 0.30 M, long stems 0.29 M. Counting whole songs instead would cost the gate minutes a project, so the counts are of that stretch. The project cases are left out of the soak and of the bench's coarser golden test.
- **On the device:** the plan checks device deadlines once per phase. These projects are what that run plays (step 0.4).

**Fizz targets by kind:**

| Kind | Target | Today |
|---|---|---|
| Clean oscillators: saw, pulse, tables | -70 dB at C5 and C6, C7 reported | Poly -78 and -84; Analog -54 and -48 |
| Hard sync, wavetable warps, FM feedback up to 5 | -60 dB at C5 and C6 | Table sync -72 and -85 (step 1.1; was -45 and -39) |
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

### What each limit is judged on (step 0.4)

Each first limit is checked on named bench cases (`rust/mpfree-bench`), counted per 128-frame block by the instruction gate on CI and timed on the MPC in the device bench round (`tools/mpc-one/host_bench_mpc.py`: alone, then split over cores with `--lanes`). The projects of step 0.6 give the full-project view. Counts below are today's, in thousands of x86-64 instructions a block (`instructions.txt`); they decide nothing about the device by themselves.

| Step | Cases (today, k instructions a block) |
|---|---|
| 1.1 Sync | Poly Table, 8 voices, sync 50 (334) against Poly Table Init (247); Wavetable Init, 8 voices, sync 50 (413) against Wavetable Init (362). Sync off must not move the Init cases |
| 1.2 Analog | Analog Init (265), Poly Brass (203), Warm Pad (321) at 8 voices; Worn Cards at 16 (466) |
| 1.3 Squelch | Squelch Square Acid (64) and Acid Line (59) |
| 1.4 Sampler read | Sampler, 20 pads (1,435); the x16 held cases (547 to 1,774); plain copies: the drum kit cases (42 to 79), which must not move |
| 2.1 Poly's drive | Poly Init, 8 voices, drive 24 dB (330) and distortion 24 dB (335) against Poly Init (194). Today's drive already costs 1.7 times the plain voice, more than the first limit of 1.3 times, so step 2.1 has to make it cheaper or bring the trade to the owner |
| 2.2 Synth filters | The Analog cases above and every Poly preset case |
| 2.3 FX cost | Drive Eco 2x (203) and HQ 4x (430), Amp (320, 342), Fuzz (386), Character's Iron (120) and Soften (97), EQ HQ 2x (188), Multi filter Hard (80), drive (144) and ladder (92), Lo-fi (222), Ring mod (13), Rotary (108), Limiter with true peak (98), Master safety (30) |
| Every step | The seven "Project:" cases: dense synths (1,332), distortion chain (1,599), granular drums (742), long stems (291), bass into the limiter (296), fast automation (572), all at once (4,215) |

The four cases this step added (sync on Poly's Table wave and on Wavetable, Poly's drive and distortion) have golden fingerprints and instruction baselines like the rest. The device run is the next device bench round, with its own go; nothing here runs on the MPC.

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
