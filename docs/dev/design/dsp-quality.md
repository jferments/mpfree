<!--
covers: rust/mpfree-fxlab/src/readers.rs rust/mpfree-app/src/rate_probe.rs rust/mpfree-synth/src/table_osc.rs rust/mpfree-synth/src/analog.rs rust/mpfree-synth/src/squelch.rs rust/mpfree-synth/src/dsp.rs rust/mpfree-synth/src/poly.rs rust/mpfree-sampler/src/engine/render.rs rust/mpfree-sampler/src/engine/voices.rs rust/mpfree-sampler/src/engine/follow.rs rust/mpfree-sampler/src/bandlimit.rs rust/mpfree-stream/src/resample.rs rust/mpfree-stream/src/cursor.rs rust/mpfree-fx/src/oversample.rs rust/mpfree-fx/src/adaa.rs rust/mpfree-fx/src/resample.rs rust/mpfree-fxlab/src/synth.rs rust/mpfree-fxlab/src/sampler.rs rust/mpfree-synth/src/bass.rs rust/mpfree-fxlab/src/robust.rs rust/mpfree-fxlab/src/reference.rs rust/mpfree-fxlab/src/filters.rs
checked: 2026-11-23 a2fecc1c
-->
# DSP quality: fizz, filters near the top and the sample rate

**Status:** plan approved by the owner on 2026-11-22. Phase 0's measures, reference renders and full-project check are built; the rate probe's code is written and, like the device timing, has not run on the MPC. Phase 1 is built: step 1.1 changes the sync warp's sound on the table oscillator ([Step 1.1: sync](#step-11-sync)), step 1.2 Analog's oscillators ([Step 1.2: Analog's waves](#step-12-analogs-waves)), step 1.3 Squelch's square ([Step 1.3: Squelch's square](#step-13-squelchs-square)), step 1.4 how sample voices read their sounds below two semitones up ([Step 1.4: the sampler's read](#step-14-the-samplers-read)), step 1.5 the levels of 21 presets ([Step 1.5: levels and DC](#step-15-levels-and-dc)), step 1.6 measures every pitched read against one standard ([Step 1.6](#step-16-one-standard-for-pitched-reads), [Pitched reads](pitched-reads.md)) and step 1.7 makes a pad's read wrap round its loop and gives the streaming cursors the 8-point read ([Step 1.7](#step-17-loop-wraps-and-the-cursors-cheap-read)). The owner's blind A/Bs of 1.1 and 1.4 were ties, so both are kept on their numbers; 1.5's levels were approved by ear; the owner chose to merge 1.2 and 1.3 without their blind A/Bs (23 Nov: merge now, fix later if needed). Phase 2 has begun: step 2.1 runs Poly's drive and distortion at twice the rate with second-order antialiasing ([Step 2.1: Poly's drive](#step-21-polys-drive)), the cleanest of three versions, which the owner chose on its cost (23 Nov); step 2.2 makes Analog's and Bass's ladders whistle in tune and Poly's filter move smoothly within each control step ([Step 2.2: synth filters](#step-22-synth-filters)), and leaves the filters near the top (Y1a) open for a choice. Step 2.3A found where the oversampled effects' time goes ([Step 2.3A](#step-23a-where-the-fx-time-goes)), and the first part of 2.3B moved Amp, Fuzz, the Exciter, Cloud's Eco and EQ HQ onto unrolled half-bands: the same output to the bit for 3 to 17 % fewer instructions ([Step 2.3B, part 1](#step-23b-part-1-the-half-bands-bookkeeping)). Design doc with both outside reviews and the answers to them: "DSP quality" (https://claude.ai/artifact/5jsLgS5ZaRoHzpvVbEUgZ5). Related pages: [FX quality](fx-quality.md), [Synth quality](synth-quality.md), [Drive and Character](fx-drive.md), [Synth library: architecture and roadmap](synth-architecture.md), [Disk streaming](disk-streaming.md), [Profiling](profiling.md).

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
| Squelch square | -48 / -39 dB | the shaper that turns the saw into a square has no ADAA (it already runs at 2x); -84 / -75 dB since step 1.3 |
| Poly drive 24 dB / distortion 24 dB | -50 / -45 and -45 / -40 dB | the curve runs per voice at 1x; -77 / -72 and -77 / -72 dB since step 2.1 |
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
| **1.4** Sampler read | A cleaner read for small pitch changes and pitch-downs. Candidates: 6- and 8-point windowed sinc from one shared table, an optimal 4-point polynomial. Tests at one and two semitones each way, a fifth and an octave down; the read's own frequency response; switching reads mid-note without a click. The plain copy at pitch and Vintage (`Pad::vintage`) stay | `mpfree-sampler/src/engine/render.rs`, `voices.rs`, `bandlimit.rs`, with the sampler refactor | S1 and Y5 -50 dB on every row, level within 1 dB; revised with the owner's choice of the 8-point read (23 Nov) to the 14 of 17 rows it meets |
| **1.5** Levels and DC | The presets still failing the level and DC rows, fixed in one pass, keeping each preset's intended loudness and tone | factory presets | L2, L3 met; changed presets on a listening page |
| **1.6** One standard for pitched reads | One quality standard for every read that plays audio at another speed: the sampler's reads, the grain reads, the streaming cursor and the stretch: images, passband, folds, loop wraps and edges, cost, each tied to fxlab rows. The readers keep their own code; any difference between them is chosen. The 8-point kernel is offered to the streaming cursor in place of its Hermite choice | `docs/dev/design/pitched-reads.md`, `mpfree-fxlab` (`readers.rs`), with the grain and disk streaming threads | Every reader measured against the standard; each gap listed with its owner |
| **1.7** Loop wraps and the cursors' cheap read | Step 1.6's gaps 1 and 4: a pad's reads take their neighbours from round its loop, not the recording past its end; the streaming cursor's Hermite gives way to the pads' 8-point read, from one shared table | `engine/voices.rs`, `render.rs`, `follow.rs`, `bandlimit.rs`; `mpfree-stream` `cursor.rs`, `resample.rs` | Q4 −100 dB on every pad read; the cursor's read measured against the standard; cost on the bench |
| **2.1** Poly's drive and distortion | Per voice, as now, one algorithm per patch setting, the same for every note. Candidates: second-order ADAA at 1x; 2x round the shaper only with first-order ADAA (Drive's Eco half-bands) | `poly.rs`, `dsp.rs`, `mpfree-fx/src/adaa.rs` | A1 drive and distortion -60 dB at C5 and C6; Y8 |
| **2.2** Synth filters | Fix only what Y1 and Y2 measure as wrong; a filter's character is not a defect. Suspects: Analog's ladder taking its saturation from the previous sample; the 0.45 x rate cutoff cap in `FilterCoefs::new` | `analog.rs`, `dsp.rs`, others as found | Y1a to c, Y2a and b; blind A/B |
| **2.3A** FX profile | Where the oversampled effects' time goes: half-bands against everything else | `mpfree-bench`, device timing | A report |
| **2.3B** Half-band prototype | One SIMD kernel for the IIR half-bands, bit-exact with today's loop, kept only if the whole effect gets faster on the device | `mpfree-simd`, `mpfree-fx/src/oversample.rs` | Whole-effect device time; every ratchet row and golden file unchanged |
| **2.3C** Iron and Soften | Back at the base rate, if Y8 and the owner's ears agree | `character.rs` | Y8; blind A/B |
| **3.1** Keep the original | A loaded file at another rate keeps pointing at its original, never only at the 44.1 kHz copy, and no sound's decoded audio is held twice | sampler load path, with disk streaming | A test |
| **3.2** High-rate playback | Automatic, no setting: at or above its own pitch a voice plays the prepared 44.1 kHz copy; pitched down, it reads the original through the band-limited read when the memory or streaming budget admits it, otherwise the copy, and that fallback is counted, logged and shown | `mpfree-sampler`, `mpfree-stream` | Y6; fallbacks shown |
| **3.3** Extreme transposition | Clean octave and deeper drops of bright sounds, and a fifth down near the top: step 1.4's tracked misses. Candidates: a longer kernel only for voices pitched more than a fifth down, and prepared copies through step 3.2's machinery | `bandlimit.rs`, `engine/`, with the sampler, grain and disk streaming threads | S1 and Y5 -50 dB on all 17 rows, within a cost the owner approves then |
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

Built, and merged without the owner's blind A/B at the owner's word (23 Nov: merge now, fix later if needed); the A/B page stays open. Analog's oscillators (`analog.rs`) were band-limited with a polyBLEP placed a sample late. They now use a 32-sample band-limited step, chosen by two experiments.

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

### Step 1.3: Squelch's square

Built, and merged without the owner's blind A/B at the owner's word, as 1.2 was. **Wave** Square is Squelch's saw through a shaper, `0.7 tanh(6 (saw + 0.1))` in its Padé form (above). The shaper took the band-limited saw, and its gain of 6 sharpened the saw's smoothed reset back into a hard edge, so the square folded at -48 / -39 / -27 dB at C5 / C6 / C7 even at twice the rate.

- **What was built** (`squelch.rs`, `sample`): the shaper takes the plain saw. Near its reset the plain saw sits in the shaper's flat ends (above 0.4 and under -0.6), so the plain square has a clean jump of -1.4 at the reset and is smooth everywhere else (the Padé curve meets its flat ends with zero slope). The jump gets the saw's own polyBLEP, scaled to its height: 0.7 times the saw's. Wave's blend of saw and square is unchanged, and so is the square's shape.
- **Compared with the plan's first-order ADAA** on the shaper (the antiderivative above, in 64-bit, the saw blended through the same half-sample delay): ADAA takes the square to -55 / -48 / -38 dB of Y8 fizz, the plain saw and the stepped jump to -79 / -72 / -62. The plain saw needs no state, no logarithm and no delay, so it was built instead.

| Measure | Before | After |
|---|---|---|
| A1 Squelch square, C5 / C6 / C7 | -48.4 / -39.2 / -27.1 dB | -84.4 / -75.0 / -62.3 dB |
| Y8 Squelch square fizz, C5 / C6 / C7 | -43.1 / -36.0 / -26.0 dB | -79.3 / -72.4 / -61.9 dB |
| Y8 8x reference against 16x, C5 / C6 / C7 | -55.2 / -47.9 / -40.2 against -61.3 / -54.0 / -43.8 dB | -89.0 / -91.0 / -88.3 against -88.9 / -91.2 / -88.7 dB |

The target was -60 dB at C5 and C6; C7 meets it too. The square's references now converge, so its three cases join `tests/reference.rs`'s list. Square Acid's other ratchet rows moved by a thousandth of a dB except its start step, -131.6 to -126.4 dBFS (past its 1 dB tolerance, still far under hearing); no other preset uses the square. The project renders are bit-identical.

- **Cost** (instruction gate, against the build before; limit 10 % more): Squelch Square Acid 64.8k to 64.4k a block (-0.6 %), Acid Line and Distorted -1.1 % (the same code laid out a little better).

### Step 1.4: the sampler's read

Built. Below two semitones up, a sample voice read its sound with 4-point Hermite interpolation, whose images of the sound's top folded back into hearing (Y5: a semitone down at 9 kHz out -28.9 dB, a fifth down at 13.5 kHz out -3.4 dB). The owner chose an 8-point read within the 15 % limit (23 Nov, after the plan's third review), and that is what was built:

- **The kernel** (`bandlimit::near_kernel`): a Kaiser-windowed sinc over 8 frames, cut at 0.455 of the sound's rate (20.1 kHz at 44.1 kHz), β 5. It is tabled at 256 rows a frame, each row's weights and their steps to the next, read with a straight line between rows (16 KB); every row sums to one. A test holds each row to the exact formula within 1e-7 and any position within 1e-5 a tap (`the_near_table_matches_the_exact_kernel`).
- **Why this kernel:** with the same runtime code, the Kaiser design meets 14 of the 17 rows below and stays flat within 0.07 dB to 10 kHz. A least-squares design meets 15 but dips 0.5 dB near 5 kHz all the time; one held flat meets 13 or 14; Blackman-Harris 7 or 8; 12 taps meet 15, at a cost the limit doesn't allow for 16-bit sounds.
- **The read, once a block** (`render::Read`): the plain copy at the sound's own speed (unchanged), the 8-point read, Vintage (`Pad::vintage`, Hermite at every pitch, as before) and the band-limited read past two semitones up (unchanged). The 8-point read takes a whole frame as it is when the voice plays at its sound's own speed (`Voice::on_frame`), found once a run, since at one step it can't change.
- **Changing read mid-note:** on a whole frame the 8-point read is not the frame (its window takes 0.9, 2.5 and 3.7 dB off 15, 18 and 20 kHz there). So when a voice's read changes (onto or off its own speed, past two semitones up), the new read fades in over the old across 256 frames, about 6 ms (`Voice::xf_left`, `render::faded`). The plain copy's side is the band-limited read unstretched: exact on whole frames, flat to about 19 kHz, with no phase error between frames. The fade's frames run on their own, so the others carry no fade checks. A sweep test through the sound's own pitch and the two-semitone line (`a_voice_changing_read_moves_smoothly`) holds each change to the steady read's own movement plus 0.6 dB and 3° in any millisecond; without the fade it fails.
- **Edges:** reads clamp to the region, a forward loop read past its end into the recording (until step 1.7), and a loop's crossfade reads with the same kernel. Tests: the noise outside a region never changes a render (four formats, three regions, five moves, every read); a one-frame region reads as its frame; a seamless loop reads as the unrolled sound, bit for bit.

**Numbers** (`quality/sampler.tsv`): 14 of the 17 rows this read is judged on meet -50 dB with the level within 1 dB (the Hermite read met 3). It misses where the sound's top is pitched well down: an octave down at 3 kHz out, -48.5 dB (Hermite -44.5); an octave down at 9 kHz out, -22.5 dB and 3.2 dB quiet (-7.7 dB, 3.0 dB quiet); a fifth down at 13.5 kHz out, -9.3 dB and 6.3 dB quiet (-3.4 dB, 4.6 dB quiet). Those are tones above 18 kHz in the sound, and passing them would take about 48 frames a row. At 3 kHz out, the semitone and two-semitone rows moved from -58 to -66 dB to -56 to -60 dB, the kernel's own floor, all still past -50; the ratchet was blessed with them.

**Real material** (synthesised drums, cymbals, supersaw chords, a plucked keygroup and white noise, each through a converter's low-pass; no kits). Each read was nulled against a 64-tap reference read at the engine's exact positions. What doesn't follow the reference fell from -26.3 to -44.0 dB on drums a semitone down, -22.7 to -32.9 dB on a 48 kHz loop at its own pitch, -17.8 to -28.3 dB on cymbals a fifth or an octave down, -36.9 to -46.6 dB on chords a fourth down and -14.4 to -23.8 dB on noise an octave down, and by 10 to 12 dB on each of the keygroup's notes. The sound's 14 to 17 kHz stays within 1.2 dB of the reference (Hermite: 1.9). The owner's blind, level-matched A/B (eight pairs, 23 Nov: those five, the keygroup melody, the loop under a bend and noise) was a tie in every pair, so the read is kept on its numbers, as sync was.

**Cost** (instruction gate, rustc 1.97.0, against the commit before): 16 held looping pads +2.5 %, the same from a 16-bit sound +11.4 %, the 20-pad case's 16 sample pads +2.2 %, all 20 pads +1.3 %, the filtered held pads 1.1 to 2.2 %, the drum kits within 1 % either way; pads pitched up (the band-limited read) and every other case unchanged. All within the 15 % first limit. The first try, the fade looked after on every frame inside one loop, cost the held pads +13.8 % and +22.6 %. The 16-bit sound costs more because each tap converts its sample. The ARMv7 build has no f32 vectors in stable Rust and the sampler forbids unsafe code, so if the device timing asks for it, the fix is a NEON kernel in `mpfree-simd` with a plain twin.

### Step 1.5: levels and DC

Built. Nine presets clipped the lab's 8-note chord at full velocity (L2), by 0.2 to 2.2 dB: Analog Init, Poly Brass, Square Keys and Worn Cards; Poly Init, Digital Bell and FM Keys; Wavetable Digital Bell and FM Keys. None had DC above −60 dBFS (L3).

- **One chord wasn't enough.** A chord's peak depends on what the voices are doing when it starts. Each preset was played 32 times, at other start times and after 0 to 7 short notes had died away. Analog's voice cards and free-running oscillators moved the peak by up to 4 dB, Pluck's noise by up to 4.4 dB, Drawbar's free-running wheels by up to 2.6 dB, and the Sync Sweep LFO by about 1 dB. Seven presets that passed on the one chord clipped on others. L2 now takes the loudest of eight chords (`synth::CHORD8_TRIALS`: starting from 0 to 0.77 s in, after 0, 2, 4 or 6 short notes; [Synth quality](synth-quality.md)), and every preset was also checked on the 32.
- **Only levels changed**, so tone, envelopes and every other ratchet row are as they were (DC is lower by the same amount). Every preset that clipped on any of the 40 chords, and every preset already being lowered, now peaks at −0.5 dBFS or lower on all of them.
- **Default levels.** Analog's, Poly's and Pluck's default **Level** drop to −3, −4 and −2 dB, as FM's did on 19 Nov (−9 dB), so a fresh instrument and each Init have headroom. Presets that played at the old default now say 0 dB and sound as they did: Poly Pluck, Brass, Sub Bass, Super Lead and Super Stab, and Pluck Pizzicato. Loading one of them glides the level up from the default over the first few milliseconds, as any preset's level does.
- **Poly's Table presets** keep following the Wavetable ones (`polys_table_presets_play_as_the_wavetable_ones`): each comes down by the same amount.

| Preset | Level | The one chord before | Loudest of the 40 chords | Phrase loudness, LUFS |
|---|---|---|---|---|
| Analog Init | 0 → −3 dB | +1.4 | +2.4 → −0.6 | −12.8 → −15.8 |
| Analog Poly Brass | −1 → −4 dB | +0.4 | +2.1 → −0.9 | −14.2 → −17.2 |
| Analog Warm Pad | 2 → 1 dB | −0.1 | +0.2 → −0.8 | −15.8 → −16.8 |
| Analog Square Keys | 0 → −3.5 dB | +0.8 | +2.6 → −0.9 | −17.3 → −20.8 |
| Analog Clang Bell | 0 → −1 dB | −2.4 | +0.3 → −0.7 | −19.9 → −20.9 |
| Analog Slow Sweep | 0 → −1 dB | −0.4 | +0.5 → −0.5 | −17.8 → −18.8 |
| Analog Hollow Pulse | 0 → −2 dB | −1.7 | +1.0 → −1.0 | −17.2 → −19.2 |
| Analog Worn Cards | 0 → −2 dB | +0.2 | +1.1 → −0.9 | −14.6 → −16.6 |
| Poly Init | 0 → −4 dB | +2.2 | +2.2 → −1.8 | −16.3 → −20.3 |
| Poly Sync Sweep | 1.9 → 0.9 dB | −0.4 | +0.4 → −0.6 | −19.2 → −20.2 |
| Poly Digital Bell | 1.9 → −1.1 dB | +1.7 | +1.7 → −1.3 | −16.8 → −19.8 |
| Poly FM Keys | 1.9 → 0.4 dB | +0.4 | +0.4 → −1.1 | −20.4 → −21.9 |
| Wavetable Sync Sweep | 0 → −1 dB | −0.4 | +0.2 → −0.8 | −19.9 → −20.9 |
| Wavetable Digital Bell | 0 → −3 dB | +1.7 | +1.7 → −1.3 | −17.6 → −20.6 |
| Wavetable FM Keys | 0 → −1.5 dB | +0.5 | +0.5 → −1.0 | −21.1 → −22.6 |
| Drawbar Jazz Perc | 2 → 1 dB | −0.7 | +0.2 → −0.8 | −17.0 → −18.0 |
| Drawbar Reedy | 2 → 1 dB | −1.4 | +0.2 → −0.8 | −15.9 → −16.9 |
| Pluck Init | 0 → −2 dB | −3.3 | +1.0 → −1.0 | −25.7 → −27.7 |
| Pluck Nylon | 1.5 → 1 dB | −1.6 | 0.0 → −0.5 | −25.2 → −25.7 |
| Pluck Harp | 0 → −1 dB | −1.6 | 0.0 → −1.0 | −24.8 → −25.8 |
| Pluck Banjo | −0.5 → −2.5 dB | −1.6 | +1.3 → −0.7 | −24.8 → −26.8 |

Peaks are in dBFS. Analog's phrases now sit at −15.8 to −21.9 LUFS, still among the library's loudest. Left as they are, because they meet the row on all 40 chords: Drawbar Hollow, Combo Perc, Init, Church, Combo Reed and Gospel (−0.1 to −0.4 dBFS), Pluck Pizzicato and Oud (−0.2, −0.3), Poly Table Init and Super Stack (−0.2, −0.5), and Wavetable Init (−0.1).

- **Reblessed:** `quality/synth.tsv`, with 193 L2 rows (the new measure: higher or the same on every preset left alone) and 12 DC rows (lower); `golden/cases.txt`, 23 renders of these presets, of Poly's and Analog's defaults, and of three Poly presets whose level now glides up from the new default as they load.
- **Cost:** none. The level is the multiply it always was.
- **Listening:** [preset levels page](https://claude.ai/artifact/PNxQqP6oGhWWAjQde4zASv), each changed preset before and after, with the lab's phrase and then an 8-note chord at full velocity, for Jay to check that each new level still suits the sound.
### Step 1.6: one standard for pitched reads

Written and measured: [Pitched reads](pitched-reads.md). Every read that plays audio at another speed (the sampler's pads, its granular pads, the grain engine, the streaming cursor and the stretch) is measured the same way by `mpfree_fxlab::readers`, with its own ratchet (`tests/readers.rs`, `quality/readers.tsv`, about 10 s in a debug build): images (Q1) and passband (Q2) on the S1 and Y5 grid, folds (Q3), and loop wraps (Q4). The sound thread's reads are held to what the 8-point pad read meets (−50 dB, 1 dB to 9 kHz out); the stretch, off the sound thread, to −70 dB and 0.5 dB. The Vintage pad and the streaming cursor's Hermite (until step 1.7) are exceptions by choice, measured with no target.

- **Meet the standard:** the pad's reads on images, passband and folds; the streaming cursor's band-limited reads (8 and 16 crossings) on everything, loop wraps exact; the stretch on everything.
- **Gaps, each with its owner:** a pad's loop wrap reads past the loop's end (−52 dB on a 1 kHz tone when the loop ends where the sound does; fixed in step 1.7); the sampler's grains at and under two semitones up keep the Hermite read (11 of 20 image rows; DSP quality, once the cost is known); the grain engine reads Hermite at every speed and folds when pitched up (DSP quality, after the sampler's grains); the streaming cursor's Hermite (replaced by the 8-point read in step 1.7, at the disk streaming thread's request); octave drops of bright sounds on every read (step 3.3).

### Step 1.7: loop wraps and the cursors' cheap read

Built. Two of step 1.6's gaps.

**Loop wraps (gap 1).** Near a loop's end, a pad's reads (the 8-point read, Vintage's Hermite and the band-limited read, its octave copies too) took their neighbours from the recording past the loop's end, and once round, from the recording before its start. When the loop ends where the sound does (a recording cut at its loop's end) the read clamped at the last frame and every wrap ticked (Q4: −52 dB on a 1 kHz tone, worse on brighter sounds); when sound follows the loop, the wrap mixed in audio the loop never plays.

- **The fold** (`Voice::fold`): each read takes the frames it looks at as the loop plays them. Past a forward loop's end, the frames from its start; before its start once the voice has been round (`Voice::wrapped`), its last frames; a ping-pong loop mirrors them about the points where the voice turns. A crossfaded loop is left as it was: its blend reads the frames either side of its ends, which is what it plays. A voice short of a loop start moved on by an edit hasn't been round that loop, and the read across a jump into a moved loop takes the recording before the jump and the loop after it (`follow_loop`).
- **Tests:** a forward loop ending with its sound reads bit for bit as the loop unrolled at −7, −0.5, +1 and +7 semitones, Vintage too; a ping-pong loop as the loop mirrored, to 2e-6; a loop start moved past a voice that has been round plays on with no step. Each fails without the fold.
- **Q4** now loops 100.6 periods (its seam a step) against the loop unrolled, so a read taking the wrong side of the seam shows: every pad read, Vintage included, is exact (−140 dB, the measure's floor), as the streaming cursors are.
- **Cost:** the reads check one range instead of two (`Voice::fast_lo`, `fast_hi`, set once a run), and only reads at the edges fold. Against step 1.6, the sampler's bench cases moved −1.0 to +0.9 % (16 held pads −1.0 %, pitched up +0.9 %, the 20 pads −0.4 %); the bench and project renders are unchanged.

**The cursors' cheap read (gap 4).** The streaming cursor's Hermite read is gone; its cheap choice is the pads' 8-point read (`Read::Near`), from the same table. The table moved to `mpfree_stream::resample::NearKernel`, so the sampler and the stream share one (the sampler now depends on `mpfree-stream`; the stream depends on nothing of the sampler's), and it is bit for bit the table the pads read before.

- At the file's own speed or slower a cursor takes the pads' weights for the same fraction and sums them the same way, so the lab measures the two within 0.005 dB of each other; faster, the same kernel stretches with the speed up to twice, as `Read::Sinc8` does (`Kernel::near`).
- **Lab:** images 20 of 20 (worst −55.2 dB) and passband 14 of 14, as the pads; loop wraps exact. Folds miss one row: an 18 kHz tone a fifth up at −30.8 dB, against −50 (Hermite −2.4, `Read::Sinc8` −54.9); 15 kHz an octave up −52.9.
- **Bench**, 64 looping voices a block: 1.84 M instructions at 0.75 times, 3.71 M at 1.5 times, against Hermite's 1.11 M and `Read::Sinc8`'s 6.41 M at 1.5 times. Nothing picks the cheap read today; the disk streaming thread decides when the sampler may.
- Every cursor test that used Hermite to land on a whole frame now uses the default read: exact at speed 1 and on whole frames slower, within half a frame's step of the marked ramps faster.

### Step 2.1: Poly's drive

Built. The owner chose the cleanest of the versions measured, on its cost (23 Nov, the card "Pick how Poly's drive and distortion stop fizzing on high notes": "Cleanest, a little more CPU"). Poly's **Drive** (into the filter) and **Distortion** (after it) were a `tanh` per voice at the base rate with first-order ADAA (19 Nov), which folded a saw driven 24 dB at -50 / -45 / -38 dB (C5 / C6 / C7) and distorted 24 dB at -45 / -40 / -35, and whose averaging took 4.8 dB off at 10 kHz and 12.7 at 15 kHz.

- **The curve** (`dsp::soft_clip`): a cubic soft clip, `x - 4x³/27` up to ±1.5 and ±1 beyond, with no corner (its slope comes down to 0 at the knee). Pushed, it plays as `tanh` does (a sine's odd harmonics within 1.5 dB of `tanh`'s from 12 dB of drive up); under about 9 dB it is a little cleaner, mostly a third harmonic. Its antiderivatives are polynomials, so second-order ADAA needs no `exp` or `ln`.
- **The antialiasing** (`dsp::DriveAa`): the curve averaged twice over the last three inputs (second-order ADAA: Bilbao, Esqueda, Parker and Välimäki, IEEE SPL 2017), at twice the rate between two polyphase IIR half-bands (three first-order all-passes in two branches, `DRIVE_HB`, from `mpfree_fx::oversample::iir_halfband(3, 0.05)`: flat within 0.01 dB to 20 kHz and 35 to 45 dB down above 24.1 kHz, enough here because the ADAA has already taken most of what folds). Between the knees the average is a polynomial in the three inputs' sum, squares and cubes (`soft_adaa2_poly`); past one knee it is ±1; only a sample whose inputs straddle a knee takes the divided differences, in 64-bit (`soft_adaa2_mixed`). The driven sound is about two frames late, flat within 1.5 dB to 10 kHz and 3.7 dB to 15 kHz. A centred voice runs one side while both sides' states are the same to the bit (`DriveAa::same_as`), and a voice with neither stage on skips the code.
- **The level keeper** (`DriveLevel::step_late`, Poly's alone: the Wavetable engine, which the app no longer plays, keeps `step` and renders as before). The input's power is taken a frame and a half late, as the output comes out (at the control step, from the last two inputs' squares: in the sample loop it cost about 3 % more on a driven voice); a note's first measure waits until it spans 16 frames; until then the gain keeps a quiet sound at its own level (`soft_quiet_gain`, the stage's slope at 0). Without the first two, a note that started two or three frames before a control step's end took up to 18 dB too much gain for a step behind the drive's latency; without the third, every note's first control step played at a gain of 1, up to 18 dB over its undriven level, which the old stage did too (half a millisecond 12 dB over Grit Bass's body, which set its chord's peak).
- **Compared** (the card's numbers): second-order ADAA at the base rate, the plan's first candidate, cost 11.5 % less than the old stage but missed at C6 (drive -56.6 dB, distortion -56.8 and -53.5 at C5 and C6); at twice the rate it meets every row. The plan's other candidate, `tanh` at 2x with first-order ADAA (Drive's Eco half-bands), met the drive's targets only just (-65.5 and -60.4 dB at C5 and C6) and missed the distortion's at C6 (-55.6). What the experiments showed: at 24 dB the curve's shape hardly matters (it acts as a hard clipper), the ADAA's order and the rate do; the half-band's sharpness doesn't matter from three sections at a transition of 0.05 up (two fall short), and linear interpolation up instead of a half-band reached only -60 at C6.
- **Tests** (`dsp.rs`): the antiderivatives' slopes are the curve and its first antiderivative across both knees; the polynomial form agrees with the divided differences between the knees; a steady input comes out steady from its first frame (a drive turned on mid-note doesn't bump); a low note keeps the plain curve's level within 0.05 dB and renders the same in 16-frame blocks as at once; `same_as` holds only while two drives would carry on alike; the late keeper waits and starts at the quiet gain, which is the stage's measured slope at 0; the input's power taken late is the frame-by-frame sum, to the bit the same in runs of any length as in one. Poly's block-size tests (`tests/poly.rs`) hold the voice to the same output for any block size, which a first version of the late sum, kept per run, broke.

| Measure | Before | After |
|---|---|---|
| A1 saw, drive 24 dB, C5 / C6 / C7 | -50.4 / -44.5 / -38.0 dB | -77.3 / -72.0 / -67.4 dB |
| A1 saw, distortion 24 dB, C5 / C6 / C7 | -45.1 / -39.9 / -34.8 dB | -77.5 / -71.8 / -66.7 dB |
| Y8 fizz, drive 24 dB, C5 / C6 / C7 | -38.7 / -35.6 / -32.3 dB, the reference not converged | -66.9 / -63.3 / -59.8 dB, against references at -88.1 / -87.0 / -83.5 that now agree with 16x |
| Y8 fizz, distortion 24 dB, C5 / C6 / C7 | -36.1 / -33.2 / -29.8 dB | -64.8 / -61.5 / -58.2 dB (references -86.9 / -85.1 / -81.8) |
| Y8 harmonics against the reference, drive, C5 / C6 / C7 | 5.9 / 5.9 / 6.3 dB | 7.5 / 7.1 / 10.8 dB |
| Y8 harmonics against the reference, distortion, C5 / C6 / C7 | 2.7 / 2.6 / 1.5 dB | 1.2 / 0.9 / 6.0 dB |
| A quiet sound's top through 24 dB of the stage, 10 / 15 / 18 kHz | -4.8 / -12.7 dB (18 kHz not measured) | -1.5 / -3.7 / -5.6 dB |

Both targets (-60 dB at C5 and C6) are met, and C7 too. Y8's 8x reference now agrees with the 16x one on the three drive cases, so 60 of the 96 cases converge (57 before), and the three join `tests/reference.rs`'s list. Against those references the drive's harmonics are a little further off than the plain saw's (6.4 / 6.5 / 6.2 dB, which is the filter's brightness at 44.1 kHz, step 2.2's question), most at C7, where one harmonic of the drive is 10.8 dB off and of the distortion 6.0: the antialiasing's own loss at the top is the likely cause, not yet traced. On a loud sound the top loses less than on a quiet one (at full level, -0.4 dB at 10 kHz and -1.1 at 15), since the curve makes its harmonics afresh. The ratchet rows of the six presets with drive or distortion moved with the new curve and the keeper's start, re-blessed in `quality/synth.tsv`: 8-note chord peaks Acid -8.7 to -8.5 dBFS, Grit Bass -9.8 to -11.5 (its start burst gone), Growl Bass -14.0 to -12.5, Lead -9.4 to -9.2, Singing Lead -12.5 to -13.7, Sync Sweep -0.6 to -1.6, all under full scale; first-frames step Grit Bass -48.2 to -78.4 dBFS and Growl Bass -88.6 to -118.2; small speakers Grit Bass -5.9 to -5.7 dB and Growl Bass 1.6 to 1.8; single notes across the keys Acid 8.1 to 8.5 dB; the velocity rows by at most 0.07 dB (the square-law fit, Sync Sweep) and 0.07 octave (tone, Acid). The bench fingerprints of the eight Poly cases with drive or distortion moved (Grit Bass, Acid, Lead, Singing Lead, Sync Sweep, Growl Bass, and Init with each), five of them past the golden file's tolerance, and were re-blessed in `golden/cases.txt` alone; of the project renders, distortion chain and all at once moved, the rest are bit-identical.

- **Cost** (instruction gate, thousands of x86-64 instructions a block, against the build before; first limit 1.3 times the plain voice): Poly Init, 8 voices, drive 24 dB 330 to 378 (+14.6 %) and distortion 24 dB 335 to 380 (+13.5 %), 1.9 times the plain voice (195; 1.7 times before), over the first limit of 1.3 times: the owner took that cost for the cleanest version (the card showed +14 %). The factory sounds with drive or distortion cost 8 to 13 % more (Grit Bass +7.8 %, Singing Lead +8.9 %, Acid +10.3 %, Growl Bass +10.5 %, Lead +11.1 %, Sync Sweep +13.0 %); every other Poly case moved by -0.2 to +0.4 % (Init +0.3 %). `instructions.txt` takes the eight moved cases at this machine's counts scaled to CI's. Device timing (step 0.4) is the real check.

### Step 2.2: synth filters

In progress. Two of Y1 and Y2's misses are fixed; the third, the filters near the top at 44.1 kHz (Y1a), is open below.

**Analog's and Bass's ladders sing in tune** (`analog.rs`, `bass.rs`; Y2b). Both ladders sang flat at Resonance 100: Analog's 181, 132, 48 and 7 cents at 110 Hz, 440 Hz, 1.76 kHz and 7.04 kHz, Bass's 190, 162, 90 and 20.

- **Why.** Not the first suspect, the stages' saturation taken from the previous sample: solved on the current sample (iterated to convergence), and in continuous time (RK4 at 400 samples a cycle), the ladder sings 200 cents flat at every pitch. With nothing but the stages' own `tanh` to hold it, a singing ladder grows until the stages saturate enough to stop it, and at that level each `tanh(d)/d` slows its stage: the cutoff drops by about 12 %. The previous-sample shares only hid part of it at high notes. A transistor ladder's `tanh(u) - tanh(v)` stages sing 10 cents flat in the same test, and a ladder whose only saturation is at its input sings in tune.
- **The fix.** The feedback is straight up to `K_STRAIGHT` (3.8); what Resonance adds past it, the last few percent where the filter starts to sing, goes through Poly's cubic `soft_clip` scaled to `FB_SING` (0.012), as a resonance amplifier limits what it gives back. A singing ladder then settles where that part bends, about 0.07 at the output stage, where the stages' `tanh` hardly slows them. Below Resonance 93 nothing changes, to the bit: every factory preset (the highest is at 65) renders as before.
- **What it does.** Both now sing within 7 cents from 110 Hz to 7 kHz (Analog -7.0 / -6.7 / -5.1 / -1.0, Bass -7.1 / -6.9 / -6.1 / -3.2), at a steady level. The whistle is quieter and even across the keys: Analog's -10.0 to -11.1 dB (the probe's level at the oscillators' scale), where it was +4.7 at 110 Hz and -3.1 at 7 kHz; Bass's -6.9 to -7.7, where it was +8.0 to +0.3. That level is set by the stages, not by a choice: any limit that keeps the pitch within 10 cents holds the output stage near 0.07, and a louder whistle goes flat again (at 0.12 it is 19 cents flat).
- **Compared.** Limiting the whole feedback, as the sampler's Ladder does (`u = x - c·soft_clip(k y4 / c)`), sings as well in tune, but it bites at every resonance: on the synth ratchet twelve Analog and Bass preset rows moved past their tolerances (Resonant Sweep's small-speaker level -1.2 to -4.7 dB, Funk Bass's -2.9 to -4.3, Unison Bass's -6.2 to -8.0), so it was not kept. A Padé curve in place of the cubic needs a lower limit for the same pitch and a division a sample.
- **Y1b** is unchanged (it measures Resonance up to 90). Y1c's audio-rate resonance sweep, which runs Resonance up to 100, peaks at -10.5 dB (Analog) and -11.1 dB (Bass) against the settings held, from -15.7 and -21.0: the held settings at 100 no longer ring as loud.

**Poly's filter moves smoothly** (`dsp::FilterGlide`, `Filter::run_moving`, `poly.rs`; Y1c). Poly worked out its filter's coefficients once a control step and held them, so a fast filter envelope moved in steps of 16 frames and added a zipper, 13 to 21 dB in the 8 and 16 kHz bands. The coefficients now move in a straight line across each step, from where the last step left them to where this one sets them, as Analog's cutoff does; a step that sets the same coefficients holds them and costs nothing extra, and a fresh note jumps to its own. They move every second sample, two samples' worth at a time: what that leaves sits at the Nyquist frequency, the measures hardly tell it from moving every sample (within 0.04 dB), and it costs less than half as much (moving every sample cost Poly 5 to 7 %). A step that moves only the cutoff leaves the resonance's coefficients alone, and the ladder takes `1/(1 + g)` as `1 - G` and its `1/(1 + k G⁴)` along the same line, so a moving sample costs no division. A buffer cut anywhere, even mid-pair, gives the same samples as one run. Stepping noise on the fast envelope: LP 24 4.9 to 0.3 dB, LP 12 20.6 to 0.02, BP 12 19.9 to 1.8, HP 12 14.5 to 0.4 (target 3 dB); on the LFO 1.4 to 3.0 dB to 0. Every move stays bounded (the worst peak over the held settings 2.6 dB, target 6). A few peak rows move by up to 0.35 dB the wrong way within that target (LP 24 audio-rate cutoff 0.02 to 0.13 dB, LP 12's 0.25 to 0.35, HP 12 envelope -0.62 to -0.27, HP 12 audio-rate cutoff 0.98 to 1.12) and were blessed. Wavetable and Keys keep held coefficients.

**Checks.** Every factory preset of Analog and Bass renders as before to the bit (all 50 hashed on 2.5 s of notes from C1 to C7 against step 2.1's build): none sets Resonance past 93, and below it the ladders run the code they ran before, chosen once a block (`ota::<SING>`, `Ladder::run::<SING>`). The synth ratchet passes unblessed, and so do the bench's golden files. **Bass's whistle and drive:** the ladder on its own sings within 7 cents of the cutoff, as at **Drive in** 0; the oscillators pushing it slow its stages, so a voice's whistle sits 112 cents under the cutoff at the default 20 and 231 at 100, steady across the keys (before, at 20 it wandered between 128 and 262 cents under). That is a pushed ladder's character, kept. Analog's whistle, set on a harmonic of the note, locks to it in a voice (0 cents on the listening clip). A blind, loudness-matched A/B of the three clips is at https://claude.ai/artifact/96thzLD4pQFv1txdMrwKgH.

- **Cost** (instruction gate, thousands of x86-64 instructions a block, against step 2.1's build on this machine): Poly's cases +0.4 to +2.6 % (Init 195 to 201, +2.6 %; Drift Pad +2.4 %; the drive and distortion cases +1.3 and +1.4 %), Analog's +0.6 to +0.9 % (Init 243 to 245), Bass's 1.4 to 2.6 % cheaper (Init 92.1 to 90.8; its block picks the ladder's path once, which its compiled loop likes), the full projects -0.5 to +0.8 %. No case comes near the gate's 5 %, so `instructions.txt` is unchanged.

**Open: near the top at 44.1 kHz (Y1a).** Every filter that runs at the base rate misses the 1 dB target from 10 kHz up, where the bilinear transform squeezes its response toward the Nyquist frequency. At a 15 kHz cutoff (the worst resonance of 0, 0.5 and 0.9): Poly's LP 24 7.8 dB off the analog curve, LP 12 3.7, BP 12 5.7, HP 12 9.2; Analog's 5.6 to 9.2; Squelch's 16.5; at 10 kHz they are 1.5 to 3.3 dB off (Squelch 7.8). Bass, which runs at twice the rate, is 0.8 dB at 10 kHz and 1.8 at 15. Nothing here is a mistake in a filter: the cure is room above 20 kHz, the filter at twice the rate as Bass has, or the higher rates of phase 3, and that costs CPU on every voice. Measured for Poly (23 Nov, a scratch filter in fxlab): at twice the rate the response below the cutoff is 0.3 to 0.8 dB off at 10 kHz and 0.8 to 1.8 at 15 kHz (LP 24 1.7, HP 12 1.8); at four times, 0.2 and 0.4 dB at most. The cost is the filter's: Poly's LP 24 takes 48 instructions a sample at the base rate, 137 at twice (half-bands 42 of them) and 291 at four times, so a mono voice of about 196 costs about 45 % more at twice the rate and 124 % more at four times. It went to the owner as a choice, with a cheap coefficient correction as a fourth option, and the owner chose to try that first (23 Nov).

### Step 2.3A: where the FX time goes

Done on instruction counts; the device timing (step 0.4's round) has not run. Each FX bench case was run for 200 blocks under callgrind twice: once with line tables, so every instruction is charged to its source line, and once with the half-bands kept out of line (a scratch build with `#[inline(never)]` on their `run`, `up` and `down`), which gives each half-band's whole cost, its state loads, stores and flushes included. That build is within -1 to +9 % of the shipped one, so the shares below lean a little high. Thousands of x86-64 instructions a block:

| Effect (bench case) | A block | Half-bands | Their share | ADAA's share | Which half-bands |
|---|---|---|---|---|---|
| Fuzz | 414 | 132 | 32 % | 24 % | list-based (`IirUp2`, `IirDown2`), to 4x, both sides |
| Character: Soften, Console, Iron, Tape | 99, 107, 123, 152 | 30 | 30, 28, 24, 20 % | 13, 18, 15, 24 % | unrolled (`IirPair<5>`), 2x, with the dry side's phase match |
| Multi filter: LP 12 Hard, ladder, LP 24 drive | 80, 96, 150 | 22, 22, 30 | 27, 23, 20 % | 0, 0, 25 % | unrolled, 2x |
| Amp: Crunch, Lead | 333, 355 | 66 | 20, 19 % | 15, 14 % | list-based, to 4x, one side |
| Drive HQ 4x | 471 | 74 | 16 % | 23 % | FIR (`Up2`, `Down2`), linear phase |
| Drive Eco 2x | 206 | 30 | 14 % | 28 % | unrolled, 2x |
| EQ HQ 2x | 207 | 24 | 12 % | 0 | EQ's own unrolled `Halves`, flushed every section |
| Cloud reverb, Eco: Plate, Hall | 251, 272 | 28 | 11, 10 % | 0 | list-based, six sections, round the network's half rate |
| Lo-fi | 230 | 22 | 9.5 % | 15 % | unrolled, 2x |
| Chain: drive, filter, delay, comp | 344 | 30 | 9 % | 17 % | Drive's Eco |
| Ring mod, Rotary, Limiter (true peak), Master safety | 13, 107, 98, 30 | 0 | 0 | 0 | none in these settings |

What it shows:

- **The list-based half-bands cost twice the unrolled ones.** A section costs 18 to 20 instructions a sample in `IirUp2` and `IirDown2` (Amp, Fuzz, the Exciter and Cloud's Eco), 8 to 9 in `IirPair` (Drive's Eco, Character, the Multi filter, Lo-fi, the Ring mod's Triangle and Square, Rotary's drive, Master safety's clipper), and 10.5 to 13 in EQ's `Halves`. The arithmetic is the same, a subtract, a multiply and an add; the rest is bookkeeping: a loop over a list with its bounds, the states in memory, and a denormal flush (`crate::flush`, a compare and a select) after every section and sample. In the line pass that flush alone is 5.6 to 7 % of Amp, Fuzz, Cloud's Eco and EQ HQ (in Cloud and the EQ part of it is their own filters'). `IirPair` was written for exactly this when Drive's Eco and Character were tuned; Amp, Fuzz, the Exciter and Cloud never moved over.
- **The unrolled ones are close to their arithmetic.** At 8 to 9 instructions a section there is little left to cut one sample at a time. Each section feeds the next, so a branch is a chain of dependent operations; what is left is running independent chains side by side, the two sides or the even and odd branches in one NEON vector. The compiler won't do that on its own: it doesn't vectorise `f32` for ARMv7's NEON (NEON flushes denormals, so it isn't IEEE exact; see `mpfree-simd`), so on the MPC every half-band and every `dot` runs one lane at a time. Whether a kernel gains depends on NEON's latency for a chain of dependent multiplies and adds, which only the device can say.
- **Drive HQ's FIR half-bands** (16 %) are linear phase on purpose, so HQ's wet and dry sum in phase. They go through one `dot` that already keeps eight independent sums in a fixed order, the order a NEON kernel of two four-lane vectors would keep, so that kernel can be the same to the bit.
- **ADAA costs as much again.** It is 13 to 28 % of every drive-type effect, more than the half-bands in most: Drive Eco 28 %, the Multi filter's drive 25 %, Fuzz and Character's Tape 24 %, Drive HQ 23 %. It works in 64-bit floats with its own `exp` and `ln`, whose lines are the hottest in `adaa.rs`. That is outside step 2.3, whose savings must leave the sound as it is; a cheaper ADAA would change the numbers, so it would be its own step, with Y8 and an A/B.

**What 2.3B does with it.** First the bookkeeping, then a kernel:

1. Amp, Fuzz, the Exciter and Cloud's Eco move onto `IirPair`, and EQ's `Halves` stop flushing every section; all of them flush their states on a grid of frames counted from the stream's start, as Drive's Eco does, so the output is the same in any block size. The arithmetic and its order don't change, so on any audible signal the output is the same to the bit (a test runs each design, five sections, four and six, against the list-based types). Built: [Step 2.3B, part 1](#step-23b-part-1-the-half-bands-bookkeeping).
2. Then one NEON kernel for the half-bands, two chains a vector, and a NEON `dot` for the FIR ones, each with its plain twin, kept only if the device bench shows the whole effect faster. Fuzz, Character, Drive, the Multi filter and Cloud run two sides, so the two sides share a vector; Amp has one side, so its even and odd branches would.

The device bench round times the 2.3 cases before and after; nothing here has run on the MPC.

### Step 2.3B, part 1: the half-bands' bookkeeping

Built. Amp, Fuzz and the Exciter run each half-band design as one `IirPair` (its up-sampler and its down-sampler: Amp's and Fuzz's first stage of five sections and second of four, the Exciter's five), and Cloud's Eco one of six sections a side; EQ HQ keeps its own unrolled `Halves` but no longer flushes after every section. Each settles its states (flushes what is under 1e-20 to zero) on a grid of frames from the stream's start: every 32 frames in Amp, Fuzz and the Exciter, as Drive's Eco does; every 16 network frames in Cloud, down-sampler and up-sampler each on its own count, since a chunk's input comes down before its wet signal goes back up; every 64 frames in the EQ, whose chunks now end on that grid. So the output is the same in any block size, and only a chain decaying into silence differs, by values under 1e-20 for a few frames.

| Bench case | Before | After | Change |
|---|---|---|---|
| Fuzz | 386.5k | 320.7k | -17.0 % |
| Amp (Crunch), (Lead, Gain 10) | 317.6k, 339.5k | 284.9k, 306.9k | -10.3 %, -9.6 % |
| Cloud reverb, Eco (Plate, Room, Hall, Cloud, Echoes) | 183.7k to 275.0k | 175.6k to 266.8k | -3.0 to -4.4 % |
| EQ (4 bands and both cuts), HQ 2x | 183.8k | 178.8k | -2.7 % |
| Project: distortion chain, all at once | 1,603k, 4,204k | 1,505k, 4,098k | -6.1 %, -2.5 % |
| Project: granular drums, dense synths | 743k, 1,317k | 735k, 1,309k | -1.1 %, -0.6 % |

x86-64 instructions a block, the instruction gate's method against step 2.2's build on this machine; no other FX or project case moves. As ARMv7 code, the MPC's, counted under qemu (a scratch example running each effect on noise at 44.1 kHz, the trace's instructions at 40 blocks less those at 10, over 30): Fuzz 456.5k to 383.1k (-16.1 %), Amp 351.7k to 314.7k and 364.4k to 327.4k (-10.5 %, -10.2 %), the Exciter 98.6k to 77.8k (-21.0 %), Cloud's Plate at Eco 295.0k to 283.6k (-3.9 %), EQ HQ 178.6k to 169.6k (-5.0 %); Drive's Eco, unchanged, 236.7k both times.

**Checks.** The same samples to the bit as ARMv7 code under qemu: each moved effect and Drive's Eco, 1,000 blocks of noise, hashed before and after. Every fxlab ratchet that covers effects (`quality`, `reference`, `robust`, `gates`) and the bench's golden files pass unblessed; the FX crate's tests pass, with the bit-exact test now over all three designs and with settling on any grid. `instructions.txt` is unchanged: the gate fails only on a rise, and its rows are CI's counts, so the drops above are for the next bless on CI's toolchain.

The rule for step 2.3 is that a saving is kept only if the whole effect is faster on the device. This part removes work and adds none (the same arithmetic, no new memory, the flush taken out of the loop), so it is kept on these counts; the device bench round times the 2.3 cases before part 2's kernels are kept or dropped.

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
- **How Y1c moves a filter.** Each filter moves as its engine moves it, a control step (16 frames) at a time. Poly's coefficients (and Wavetable's) are worked out once a step and held; the sampler's were too until the filter gap plan's step F2b, and since then glide a step a frame to each new setting. Analog's, Squelch's and Bass's cutoffs move in a straight line of the filter's gain across the step, through each probe's `glide`. The sampler's filter runs through a copy of what its voice does (`PadMirror`), held to a real pad by a test (the same to f32 rounding, with an envelope sweeping it). There are four moves. A fast filter envelope rises from 100 Hz to 12.8 kHz in 1.5 ms and falls back with a 5 ms time constant, 21 times a second. An LFO swings two octaves either side of 1 kHz at 5.4 Hz. The cutoff swings three octaves at 689 Hz, and the resonance from 0 to 1 at the same rate.
- **Y1c: every filter stays finite and bounded.** Each filter is fed noise at -6 dBFS and moved every frame and at its engine's step, at resonance 0, 0.9 and 1. Its peak is compared with the loudest the same noise gets with the settings held at 16 places along the move. No sample is infinite or NaN. The worst peak is 4.9 dB over the held one (Squelch, on the envelope's rise; target 6 dB).
- **Y1c: held coefficients zipper on a fast envelope.** Each filter is fed a steady 1 kHz tone at -20 dBFS and moved at its engine's step, then every frame. The two are compared octave band by octave band, where the output is within 60 dB of its loudest band. Each move repeats in a whole number of steps and the tone fits the window, so the spectra have no leakage, and a step's lag doesn't count, only the noise it adds. Analog, Squelch and Bass glide and stay within 2.7 dB of the smooth move on the envelope, and 0 dB on the LFO. Poly's and the sampler's held coefficients added 13 to 21 dB on the envelope, mostly in the 8 and 16 kHz bands, 32 to 46 dB under the loudest band (the 2-poles worst; Poly's LP 24 4.9 dB, the sampler's Ladder 5.2, Vowel 3.2 and Comb 1.0). On the LFO they came to at most 4.2 dB, the zipper about 60 dB under the tone. Moving the coefficients in a straight line across the step, as Analog does, is the measured fix. The sampler has it since F2b ([fx-filter.md](fx-filter.md#the-pad-filter)): its state-variable types now add 1.5 to 2.6 dB on the envelope and 0.16 or less on the LFO, its Ladder 2.7 and 0.0001; its Vowel and Comb, not yet gliding, are as they were. Poly's waits for the step that next works on it.
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
| 2.1 Poly's drive | Poly Init, 8 voices, drive 24 dB (330) and distortion 24 dB (335) against Poly Init (194). Today's drive already costs 1.7 times the plain voice, more than the first limit of 1.3 times, so step 2.1 has to make it cheaper or bring the trade to the owner (it went to the owner: [Step 2.1](#step-21-polys-drive)) |
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
