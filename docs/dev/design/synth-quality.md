<!--
covers: rust/mpfree-fxlab/src/synth.rs rust/mpfree-fxlab/tests/synth.rs rust/mpfree-fxlab/quality/synth.tsv rust/mpfree-bench/src/cases.rs
checked: 2026-11-19 95c1134d
-->
# Synth quality: the baseline, the measures and the ratchet

**Status:** measure-only baseline built 2026-11-18 (synth library stage 2). No engine changed: this page says where today's synths stand against the targets the research pages propose, and the ratchet stops any of those numbers getting worse while stages 3 and 4 improve them. Research: [What every type shares](../research/synth-topics.md) and the type pages it links. Plan: "Synth library: stock-take and research plan" (https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). **Nothing here has run on the MPC**; the cost table is the workstation's.

The method is the FX quality one ([FX quality](fx-quality.md)): each target a number with its reason, an fxlab module that measures it on signals with known answers, and a ratchet on CI. Targets in brackets on the research pages are proposals until listening and the device confirm them; here they are the bar a row is "met" against, and a row that misses one is printed, not failed.

## The report

`fxlab synth [--rate HZ] [--engine NAME] [--renders] [--out DIR]` measures every keyboard engine's factory presets (Poly, Analog, Squelch, FM, Wavetable, Keys, Drawbar, Pluck: 83 presets at the baseline; Strings' 24 joined with wave 1, Drawbar's 8 combo presets, Wind's 20 and the Choir's 25 with wave 2), each engine's plain patch, Poly's voice, FM's operators and the five drum kits, and prints one row per number with its target and whether it is met, then a count by measure; it writes DIR/synth.txt. With `--renders` it also writes each preset's listening phrase (C, E and G, then a four-note chord; two octaves down for a bass) as DIR/renders/ENGINE/NN-PRESET.wav, and each kit's tour (every pad, then two bars of a beat). The report takes about 21 s in a release build on the workstation. A factory preset that brings its own effects and macros (a sound, [Presets](presets.md#factory-presets-as-files)) is measured as the pads play it, through its effects; the K rows, which measure the engine's own voicing (decay against its label, the release drop, the pickups, the tremolo), play its engine alone, since a sound's echo or reverb lengthens a decay by design.

Every render plays 100 silent blocks first, so parameters a preset or patch sets have finished gliding, as they have when a player picks a preset and then plays (without them Poly's Decay read 19 % long: its Sustain was still gliding down). Renders run in 128-frame blocks on absolute frames, as on the MPC.

| Id | Subject | What is measured | Target | From |
|---|---|---|---|---|
| L1 | every preset | The phrase's loudness, LUFS | shown | |
| L2 | every preset | An 8-note chord at full velocity, sample peak | under 0 dBFS | P4 |
| L3 | every preset | DC over the phrase | under −60 dBFS | B8 |
| L4 | every preset | Seconds after the last note-off until under −80 dBFS | shown | |
| L5 | every preset but the organ's | Spectral centroid of the first 370 ms, velocity 127 against 20, octaves | at least 0.1 | K1, F8, P9 |
| L6 | every preset | Level (loudest 20 ms) from velocity 20 to 127; the worst error of the best-fitting square-law curve (Dannenberg, ICMC 2006) over 20, 40, 64, 100 and 127 | range shown; fit within 2 dB | K2, P9 |
| L7 | every preset | Full pressure from 0.3 s against none, level and centroid | shown | |
| L8 | every preset | Mono fold-down loss: the phrase's LUFS against (L+R)/2 on both sides | under 3 dB | P8, S3 |
| L9 | every preset | Stereo correlation of the phrase | shown | |
| L11 | every preset | LUFS spread of single notes: C2 to C6; C1 to C7 for Keys and Pluck; C1 to C3 for a bass | under 3 dB | K6, B6 |
| L12 | every poly preset | A 6-note chord against the power sum of its notes, LUFS | within 6 dB | S9 |
| L13 | every preset | Milliseconds until 3 dB under the loudest 5 ms | shown | |
| B4 | bass presets | The largest step in the first four frames of C2 | under −40 dBFS | B4 |
| B5 | bass presets | 100 to 400 Hz against 20 to 100 Hz, the worse of C1 and C2 | at least −12 dB | B5 |
| B7 | bass presets | The phrase's correlation under 120 Hz | at least 0.95 | B7 |
| K1 | Keys presets | Second harmonic against the first at C4, velocity 127 against 40 | at least 12 dB more | K1 |
| K3 | Keys and Pluck presets | T60 at C4 against the Decay label; the slope of log T60 against log frequency over C2 to C6 against the engine's law (−0.75 Keys, −0.5 Pluck) | within 20 %; within 0.15 | K3 |
| U1 | Pluck's string (Steel String's pick and brightness), 16 plucks at C3, C4 and C5, each pluck's power averaged | Position 50: the even harmonics (2, 4, 6, 8) against the odd (1, 3, 5, 7); Position 20: the 5th against the 4th and 6th; the worst note | under −30 dB | Pluck brief |
| K9 | Keys presets with tremolo | The level's modulation rate against Trem rate | within 5 % | K9 |
| T1 | each engine's plain patch, and Drawbar's combo (its flute on the 8') | Pitch of C1, C2, C4, C6 and C7 (a Blackman-Harris peak refined by the phase it turns through in a second): the worst, cents | within 1 cent (the tonewheel organ, tuned by its gears, shown) | S6, B1 |
| A1 | plain patches with saw, pulse, square, drive, distortion, wavetables, FM feedback, FM's waves under modulation; Drawbar's combo flute, saw reed and square reed | Loudest component off the harmonics, 20 Hz to 20 kHz, against the loudest harmonic, at C5, C6 and C7 | under −60 dB | P1, F3, B10 |
| O6 | Drawbar's combo, its flute on one footage of C4 at a time | The 16', 4', 2' and 1' against the 8': the worst, cents from an exact octave | within 0.1 cent | O6 |
| B2 | Poly, legato C3 to C4, Glide 200 ms | Landing pitch; time to within 12 cents (99 % of the octave) against the label | within 2 cents; within 10 % | B2 |
| P6 | Poly's amp envelope on a sine | Attack to 99 % of full level, Decay to within 1 % of Sustain, Release to 60 dB under the held level, against the labels (since 19 Nov a time means what it says; before, 99 % of a segment and Release to 1 %) | within 10 % | P6 |
| P10 | Poly's Age on a sine, 16 A4s in turn (one a card); the voice core's drift stepped every 0.1 s for 30 minutes | The notes' tuning spread at Age 100 against what the cards and the drift say; Age 50 against 100; Age 0; the drift's rms and how far it wanders | within 1.5 cents; a ratio within 0.15 of 0.5; under 0.01 cents; within 0.5 of 2 cents; under 10 cents | P10 |
| P12 | Poly's play routes, the mod wheel at the top | A route on Pitch at 100 % against an octave up; the default route (the wheel on Vibrato at 50 %) against half a semitone each way | within 1 cent; within 5 % | P12 |
| B9, P5 | Poly's LP24 and LP12 at 1 kHz | A C1 saw's harmonics filtered against the same saw with the filter open, against the analog prototype (`1/(1+s)^4`, `1/(1+s)^2`) where it is above −30 dB; where it is 3 dB a pole down against the label; full resonance on noise, where it peaks at 250 Hz, 1 kHz and 4 kHz | within 1 dB; within 0.1 octave; within 25 cents | B9, P5 |
| F1 | FM, operator 2 at ratio 4 into operator 1 | Partials at 1 ± 4n of C4 against the Bessel functions |Jn(I)|, I fitted, at operator levels 60, 75 and 90 | within 0.5 dB | F1 |
| F3 | every FM preset, its engine alone | At C6 and C7 at its brightest (every operator's envelope held at its first level, detune and the LFO's vibrato at 0, the fundamental an octave down where an operator runs at ratio 0.5), what is off the harmonics against the loudest harmonic, as A1 | under −60 dB at C6, −50 dB at C7 (a preset with partials off the harmonics by design shown) | F3 |
| F5 | FM, operator 6 alone | Feedback 7 against 0, LUFS | within 3 dB | F5 |
| F7 | every FM preset, its engine alone | LUFS spread over the 32 algorithms | under 3 dB for Glass Keys (the research's one preset); the others shown | F7 |
| F8 | every FM preset | Spectral centroid of the first 370 ms at velocity 20, 40, 64, 100 and 127; the smallest step up, octaves | 0 or more (it never darkens as velocity rises) where the tone moves at least 0.1 octave in all; shown otherwise | F8 |
| D3 | every drum pad of every kit | Centroid of the first 93 ms, velocity 127 against 40, octaves; the smallest step in that centroid over velocities 20, 40, 64, 90 and 127; the attack (the first 10 ms above 2 kHz), velocity 127 against 40 | at least 0.1; no step down; at least 6 dB | D3 |
| D4 | every pad | The same hit twice, the second's level against the first; on a kick, its pitch 0.2 to 0.4 s in | 0.2 to 1 dB; 1 to 5 cents | D4 |
| D5 | every pad | Time to 60 dB under the loudest 5 ms against the Decay label | within 10 % | D5 |
| D7 | every pad; every kit | Level (loudest 20 ms) against the kit's first kick; the pad's peak at full velocity; every pad at once, twice (32 hits), the kit's peak | within 6 dB; under 0 dBFS; shown | D7 |
| D1, D2, D6, D8, D9 | the 808 kick, the metal bank, a choke, a hit's start | Against the circuit papers and the engine's own rules: [Wave 3 targets](#wave-3-targets-drums) | | D1 |

The drums' own effects (wave 3, 19 Nov) are measured in `rust/mpfree-synth/tests/drum_fx.rs`, not in fxlab, since no factory kit uses them yet: Punch and Sustain move a hit's front and tail by 3 dB or more at any level (within 1 dB between a hit at 0 and at −12 dB), Distortion keeps the peak within 3 dB while the crest falls, the EQ's bands and the Compressor do what their labels say, and Distortion's aliasing is held at its measured numbers (101 Hz under −95 dB at 24 dB of drive, 997 Hz under −50 dB; [Synths](synths.md#each-drums-effects-and-the-kits-sends-drum_fx)). D10, the cost, is the instruction gate's four "Drums, 16 pads ringing" cases.

New analyses in `rust/mpfree-fxlab/src/synth.rs`, each tested on signals with known answers (`cargo test -p mpfree-fxlab --lib synth`): the frequency read (a sine 0.07 % off reads within 0.01 cent at C1, C4 and C7), the alias read (a band-limited saw under −85 dB, a naive one above −40 dB), the Bessel functions and the square-law fit.

## The ratchet

`tests/synth.rs` runs the whole report and compares every row that has a better direction (2,539 of them on 19 Nov, with Wind, the Choir and the drums' wave 3 rows) with `rust/mpfree-fxlab/quality/synth.tsv`, as the FX ratchet does: a number worse than recorded by more than its tolerance, a new row, or a recorded row no longer measured fails it. It renders every preset many times, so it runs in release only (ignored in a debug build) as its own CI step, about 21 s. A change that improves numbers records them with `MPFREE_QUALITY_BLESS=1 cargo test --release --locked -p mpfree-fxlab --test synth`; one made worse on purpose is blessed the same way and explained in its pull request. Adding a preset adds rows, so a preset pull request blesses too.

## Where today's synths stand (2026-11-18)

Wind (19 Nov) adds 20 presets that meet every shared row (L2, L3, L5, L6, L8, L11; [Wave 2 targets](#wave-2-targets-wind-stage-4)); the counts here are from before it, as they are from before FM's wave 2 fixes (19 Nov; [Wave 2 targets: FM](#wave-2-targets-fm)). The Choir (19 Nov) adds 25 presets that meet every shared row but L11 on six of them ([Wave 2 targets](#wave-2-targets-choir)); they aren't in these counts either. Met against rows with a target (with Strings, Bass, the Keys clav, the first six factory sounds and, 19 Nov, Poly's new presets, Worn Brass among them, and Drawbar's combo): L2 163 of 180, L3 177/180, L5 108/160 (the six tine presets that missed it meet it since the tine's rebuild, 19 Nov), L6 179/180, L8 180/180, L11 130/180, L12 126/126; B1 2/2, B3 3/3, B4 50/50, B5 31/50, B7 50/50; K1 23/23, K3 92/92, K4 11/11, K7 5/5, K9 8/8; S7 8/8; V7 2/2; T1 10/10; A1 42/72 (19 Nov: Poly's Table wave and the sync warp on both table engines added 9 rows); O6 1/1; B2 2/2; P6 3/3; P10 5/5 (19 Nov: 0.07 cents off its label, a ratio of 0.50, 0.0001 cents, 2.0 cents rms, 8.5 cents at most); P12 2/2 (19 Nov: 0.0001 cents, 0.06 %); B9 3/3, P5 2/2; F1 3/3, F5 0/1, F7 0/1; D3 19/80, D5 70/80.

- **In tune and true to theory.** Every engine's plain patch is within 0.03 cents from C1 to C7, except Keys (0.55 cents at C2) and the tonewheel organ (0.58 cents, its gear ratios); Drawbar's combo is within 0.0006 cents, its octaves exact (O6). FM's sidebands match the Bessel functions within 0.01 dB (operator level 60, 75 and 90 give an index of 0.43, 1.57 and 5.76). Poly's LP24 is within 0.16 dB of its 4-pole prototype and LP12 within 0.8 dB, the cutoff where the label says, the resonance peak within 5.3 cents. Glide lands within 0.04 cents in 1.2 % of its time. Keys' and Pluck's decays follow their labels and laws (all 88 K3 rows), and the tremolo rates are within 0.4 %.
- **Aliasing is the biggest gap.** Poly's saw and pulse (PolyBLEP) alias at −49 dB at C5, −44 at C6 and −37 at C7 (the saw's 13th harmonic folds to 16.9 kHz); drive and distortion make it −30 and −27 at C7. (Fixed for Poly 19 Nov, wave 2: from the voice core's tables its saw and pulse are at −78, −84 and −90 dB and its triangle −94 to −99; the antialiased drive is at −50, −45 and −38 and the distortion −45, −40 and −35, still short of −60.) Analog: −54, −48, −42. Squelch's saw meets −60 to C6, its square doesn't (−27 at C7). Wavetable's mips work: Init −72 at C7, position 100 −54. Poly's Table wave (19 Nov, the fold) reads the same tables the same way and measures the same to the tenth of a dB; the sync warp at 50 % is at −45, −39 and −33 dB on both engines, since the jump at each restart is not smoothed (a follow-up). FM feedback 7 sits 13 to 17 dB under its harmonics with what is off them (aliasing or the feedback's noise; the row can't tell which). (FM's feedback and modulators ease off on high notes since 19 Nov, wave 2: feedback 5 at full level is at −65 to −62 dB, and every harmonic preset under −60 dB at C6; see [Wave 2 targets: FM](#wave-2-targets-fm).)
- **Velocity is narrow.** From velocity 20 to 127 Poly's level moves 7 to 8 dB, Analog's 2 to 8, Pluck's 6 to 10 and Keys' 12 to 22, against the 30 to 40 dB the research proposes; the curve's shape is fine (81 of 83 within 2 dB of a square law). Tone follows velocity on 32 of 71 presets; on the drums only snares, claps and FM perc brightened (19 of 80 pads) until wave 3's voices, with which every pad's tone rises at every velocity step ([Wave 3 targets](#wave-3-targets-drums)).
- **Levels.** 17 presets clip an 8-note chord at full velocity (up to +3.2 dB: Pluck's mallets and Steel Drum, Poly Init +2.0, FM Init +1.9). Single notes across the keys stay within 3 dB on only 25 of 83 presets (Pluck's strings up to 9 dB). DC is above −60 dBFS on 8 presets: FM's 1:1 pairs (Glass Keys −38.5 dBFS: a 1:1 pair puts a sideband at 0 Hz), the reeds (Reed Bite −45.6) and Poly Lead (−45.1). (FM's DC blocker, wave 2, puts every FM preset under −92 dBFS.) Every preset keeps its loudness within 3 dB in mono.
- **Knobs that say one thing and do another.** Poly's Attack reaches full level in 31 % of its label (the envelope aims at 1.3 and stops at 1), and Release reaches 1 % in 85 % of its label (it aims just below zero); Decay is exact. FM feedback 7 is 3.5 dB louder than 0, and Glass Keys moves 6.4 dB across the algorithms (2.6 dB since wave 2).
- **Small speakers.** Poly Sub Bass (−37 dB) and FM Rubber Bass (−85 dB: a sine by 0.1 s at C1 and C2) all but vanish on a phone; 7 of the 16 bass presets are under −12 dB.
- **Drum decays** are within 10 % of the label on 70 of 80 pads; the Grit kit's kicks ring 24 to 30 % long.

## Voices: what each engine costs

`mpfree-bench --afford [--filter TEXT]` now plays every factory preset of every keyboard engine at 1, 2, 4, 8 and 16 voices (keys held, for the organ; a mono preset once, on a line), fits `p99 = fixed + voices × per voice` and says how many voices fit in the synth budget, with the worst block over the mean at the most voices beside it ([Profiling, What can we afford?](profiling.md#what-can-we-afford---afford)). It is not a ratchet row: costs on a shared VM are too noisy, and the instruction gate counts the bench's synth cases already. On the workstation (Xeon, 18 Nov) a voice costs about 4 µs a block for Poly Init (6.7 for Supersaw), 5 for Analog, 3 to 5 for FM, 3.5 to 7.7 for Wavetable, 5 for Keys, 1.8 for Pluck and about 1 for the organ's keys (its wheels cost the same whatever is held). The MPC's numbers come from a device round.

## Wave 1 targets: Bass, Keys and Strings (stage 4)

The bar each wave 1 pull request is judged by ([Architecture and roadmap](synth-architecture.md#7-roadmap)). The ids are the research pages' (B, K, S) and the voice core's (V). Each row joins the report and the ratchet in the pull request that builds what it measures; until then it is listed here only. "Today" is the baseline's number for the nearest thing there is. Bracketed targets on the research pages are now the bar; listening can still move one, and the page says when it does.

**The voice core (V)**, on every engine built on it and every new preset:

| Id | Measure | Target | Today |
|---|---|---|---|
| V1 | Level from velocity 20 to 127 (L6) | The preset's Velocity range within 1 dB; Keys presets 30 to 40 dB, others at least 18 dB (ours) | Poly 7 to 8 dB, Keys 12 to 22 dB |
| V2 | The square-law fit (L6) | Within 2 dB | 81 of 83 presets |
| V3 | Tone against velocity (L5) | At least 0.1 octave, every preset but the organ's | 32 of 71 presets |
| V4 | Envelope times | Attack reaches 1 dB under full level within 10 % of its label; Release falls 60 dB within 10 % of its label | Poly met (19 Nov, on the voice core): Attack reaches 99 % at 98 % of its label, Release falls 60 dB on it (were 31 % and 85 %). FM, Wavetable and Analog not yet |
| V5 | Sustain pedal (K5) | A note let go with the pedal down sounds until the pedal comes up, then releases; a key struck again under the pedal restarts its note; the same through the host's fallback for an engine that doesn't hear the pedal | No pedal |
| V6 | Note priority and legato | Last, low and high each pick the right note as keys go down and up; legato with retrigger off never restarts the envelopes | Last note only |
| V7 | Glide by rate | Covers an octave in the time its rate gives, within 10 %; lands within 2 cents (B2) | Time only |
| V8 | DC (L3) | Under −60 dBFS | 8 presets over. Poly met (19 Nov: the output's DC blocker while drive or distortion sounds; Lead from −44 dBFS, Grit Bass from −63 and Acid from −70 to the floor) |
| V9 | An 8-note chord at full velocity (L2) | Under 0 dBFS | 17 presets clip |

**Bass (B)**, on the Bass engine's plain patch and presets:

| Id | Measure | Target | Today |
|---|---|---|---|
| B1 | Pitch of a held note 100 ms in, C0 to C3, both bodies | Within 3 cents | Bass: met, 0.07 cents (synth) and 0.05 (808) at worst |
| B2 | Glide (with V7) | Lands within 2 cents; time within 10 % | Bass by rate: met, 0.002 cents, 0.3 % (V7); Poly by time: 0.04 cents, 1.2 % |
| B3 | The 808's drop | Amount and time as set, within 5 %; then settles per B1 | Bass: met, 12 semitones over 200 ms on C4 within 1.9 % and on time, then 0.0005 cents |
| B4 | The note's start | Largest step in the first four frames under −40 dBFS with start phase 0° | Bass: met on 34 of 34 (−75 dBFS at worst, 808 Kick Bass); other engines' bass presets 16 of 16 |
| B5 | Heard on small speakers | 100 to 400 Hz at least −12 dB against below 100 Hz, C1 and C2, default drive | Bass: 22 of 34. Not met by the plain subs (Sub Sine, Sub Triangle), the clean 808s, Soft Triangle and Soft Reese, which are pure low end by design, nor Distorted 808 (−16.7 dB) and Tube 808 (−25.5 dB). Amp Bass meets it through its cabinet (−2.4 dB). Other engines' bass presets 9 of 16 |
| B6 | Level across the keys | Loudness spread under 3 dB, C0 to C3 | Bass: met on 34 of 34 in L11 (0.6 to 2.9 dB), with the level rising under C3 |
| B7 | Low end stays mono | Correlation at least 0.95 under 120 Hz | Bass: met on 34 of 34 (it is mono); other engines' bass presets 16 of 16 |
| B9 | The ladder | Within 1 dB of the analog prototype at rest; resonance peak within 25 cents | Poly's: 0.16 dB, 5.3 cents. Bass's (Analog's ladder) is not measured yet |
| B10 | Aliasing at full drive | Worst alias under −60 dB at C5 | Bass saw at Drive 100: met, −63.7 dB at C5 (−58.8 at C6, −54.0 at C7); Poly saw with 24 dB drive: −42 dB at C5 |
| B11 | Cost | One voice under 5 % of a period in the worst block on the MPC; the workstation's reported | Workstation, a sixteenth-note line, instructions a block: Bass Init 93k, Reese 128k, Distorted 808 65k. MPC: device round |

**Keys (K)**, on Keys' presets, the clav's included:

| Id | Measure | Target | Today |
|---|---|---|---|
| K1 | Second harmonic, velocity 127 against 40, C4 (tine and reed: the clav's velocity brightens the whole string, which L5 reads) | At least 12 dB more | 23 of 23 (12.8 to 27.1 dB; the tines 18.8 to 27.1 since their rebuild, 19 Nov, against 12.7 to 18.3) |
| K2 | Level against velocity | V1 and V2, range 30 to 40 dB | Velocity sets the range, 32 dB from velocity 1 to 127 by default; 14 to 24 dB from 20 to 127 as measured (L6), every preset within 1.9 dB of the square law |
| K3 | Decay against pitch | T60 within 20 % of the label (the clav's shortened by its Mute, as labelled); slope within 0.15 of the engine's law | 68 of 68 |
| K4 | The end of a note | The clav drops by its Release drop (3 by default) ± 0.5 semitones at release; any damper sound is under the note's level | 11 of 11 clav presets, within 0.01 semitone (C5, read 10 to 50 ms after the release) |
| K5 | Sustain pedal | V5 | No pedal |
| K6 | Level across the keys | Loudness spread under 3 dB, C1 to C7 | 34 of 34 (0.7 to 3.0 dB, L11; the tines 0.7 to 2.3) |
| K7 | Clav pickups | Notches where the pickup's place predicts, within 5 %: the partial over the node at least 15 dB under its neighbours, C2 | 5 of 5 clav presets on one pickup (−19.6 to −34.2 dB) |
| K9 | Tremolo | Rate within 5 % of its label | 8 of 8 |
| K10 | Cost | 16 tine voices and 10 clav voices under the synth line on the MPC | Device round |
| K11 | The tine against a real tine piano | Harmonics 2 to 8 against the note, 50 and 300 ms in, over recordings of 15 keys at five velocities, rms dB: no worse than the recordings' own scatter from key to key (about 9 dB) | Init 9.1 dB, no bias (23.7 before the rebuild). Measured outside the lab, as the recordings are not ours to commit ([Electric pianos](../research/electric-piano.md#measured-tine-piano-19-nov)) |

**Strings (S)**, on the Strings engine and the Ensemble effect:

| Id | Measure | Target | Today |
|---|---|---|---|
| S1 | The ensemble's pitch movement | Slow and fast rates within 5 % of their labels; depth in cents as labelled | Ensemble effect: met, within 2 % of the label at 5 to 30 cents (frame by frame, `ensemble.rs` tests) |
| S2 | The lines' movement | 120° apart, within 5° | Ensemble effect: exact (a third of a cycle; a quarter with four lines) |
| S3 | Width | Correlation 0.2 to 0.6 with the ensemble on; mono fold-down loss under 3 dB | Ensemble effect: 0.51 (three lines) and 0.58 (four) above 300 Hz; mono fold -1.1 LU |
| S4 | Top end | The wet path 3 dB down between 6 and 9 kHz | Ensemble effect: met (-1.4 dB at 6 kHz, -5.4 at 9) |
| S5 | Cost against keys held | Flat within 10 % from 1 to 49 keys (workstation bench, then the MPC) | Not met, by design: the bank renders only the notes heard. Workstation, instructions a block: 154k for one key (most of it the Ensemble), 221k for 6, 385k for 49 (2.5 times one key); a track at rest stops running after 0.1 s. MPC: device round |
| S6 | Tuning | Every key within 1 cent in every built-in tuning; 24-TET with each note at its own pitch | Met: every key within 1 cent in every built-in tuning on two roots, 0.001 cents from C1 to C7 (T1); 24-TET within 0.01 cent |
| S7 | Crescendo and Sustain | Within 10 % of their labels | Met: −2.3 % to +0.2 % |
| S8 | Aliasing | Worst alias under −60 dB at C7 | Met: −65.5 dB at worst (Violin) |
| S9 | Chords | A 6-note chord within 6 dB of its notes' power sum (L12) | Met: −0.55 to +1.61 dB over the 26 presets |

Drawbar's combo model (19 Nov), against the [organ targets](../research/organ-wind-choir.md#4-what-to-measure) and the rows every preset gets:

| Id | Measure | Target | Today |
|---|---|---|---|
| O5 | Cost against keys held | Flat within 10 % from 1 to 49 keys | Not met, by design (as S5): the bank renders the notes heard. Workstation, instructions a block: 154k for Combo with 8 keys (three footages, both voices), 182k for Combo Reed with 8 (seven footages). MPC: device round |
| O6 | Combo footages | Octaves exact, within 0.1 cent | Met: 0 cents (the 16', 4', 2' and 1' of C4 against its 8') |
| T1 | Tuning | Within 1 cent | Met: 0.0006 cents from C1 to C7 |
| A1 | Aliasing | Under −60 dB at C5, C6 and C7 | Met: −76 dB at worst (the flute at C7); the saw reed −78.7 dB, the square −80 dB |
| L11 | One note across the keys | Under 3 dB from C2 to C6 | Met on all 8 presets: 0.7 to 2.6 dB |
| L12 | Chords | A 6-note chord within 6 dB of its notes' power sum | Met: −0.3 to +2.2 dB |
| L2 | Headroom | An 8-note chord at full velocity under 0 dBFS | Met: −0.8 dBFS at worst (Combo Reed) |

The Ensemble effect also meets the FX method's own rows: no dip as Mix goes up (within −1 to +1.5 LU of the dry), no zipper over 3 dB on any control, and no cut-off tail ([FX quality](fx-quality.md#delay), D6, D7 and D12). Built 18 Nov: in the `modulation` class it meets all of them (mix dip -0.33 LU, no bump; every control's click at least 60 dB under the tone; on insert within ±0.4 LU at the defaults and every preset).

## Wave 2 targets: FM

FM's fixes (19 Nov, [Synths](synths.md#fm-mpfree-synth-fm)): the DC blocker, the level match across algorithms, feedback and bright modulators that ease off on high notes, operator waves kept clean under modulation, and the presets re-levelled as sounds with macros. Every FM row meets its target or says why not.

| Id | Measure | Target | Today |
|---|---|---|---|
| F1 | Sidebands against the Bessel functions | Within 0.5 dB | Met: within 0.012 dB at operator levels 60, 75 and 90 |
| F3 | Aliasing, every preset at its brightest | Under −60 dB at C6, −50 dB at C7 | Met on all 9 harmonic presets (Steel Pluck −65.9 dB at C6 and Rubber Bass −54.4 at C7 the closest; most under −90). Bell Tower, whose partials sit at 3.5 and 1.41 by design, is shown only: the row can't tell its partials from aliasing |
| F5 | Feedback 7 against 0 | Within 3 dB | Not met, by design: 3.50 dB. The operator's power stays the same; feedback 7 turns rough and noisy, as on the DX7, and loudness weighs that brightness. Easing it would change the sound DX7 players expect |
| F7 | Level across the 32 algorithms | Under 3 dB for Glass Keys | Met: 2.62 dB (6.4 before). The other presets are shown: where carriers share a frequency they add or cancel by algorithm, and a modulator turned carrier changes the brightness |
| F8 | Velocity brightens step by step | Never darkens as velocity rises | Met on the 8 presets whose tone moves with velocity (Init is one sine; Drawbars is an organ) |
| A1 | Feedback 5 at full level; a saw modulated by a sine; a sine by a square at full level | Under −60 dB at C5, C6 and C7 | Met: feedback 5 −65.2, −64.9 and −62.1 dB; the saw −96.2, −94.9 and −65.6; the square −88.6, −89.6 and −91.8. Feedback 7 stays at −17 dB: rough by design, and most of what the row finds is the loop's noise |
| L1 | Level with the category | Phrase loudness within 3 LU of the category's median | Met on all 10 (bass −20.25, keys −20.37, lead −19.80, organ −16.95, pad −25.08, pluck −21.39, brass −23.41 LUFS at the median) |
| L2 | An 8-note chord at full velocity | Under 0 dBFS | Met on all 10 (−1.2 dBFS at the most, Drawbars; Init was +1.9) |
| L3 | DC | Under −60 dBFS | Met on all 10 (−92 dBFS at the most; Glass Keys was −38.5) |
| L5 | Tone against velocity | At least 0.1 octave | Met on 8 of 10. Init is one sine (no modulator to brighten); Drawbars is an organ, exempt |
| L6 | Level against velocity, the square-law fit | Within 2 dB | Met on all 10 (1.95 dB at the most, Rubber Bass). Glass Keys read 2.06 once its DC was gone (its 1:1 pairs, driven harder by velocity, made velocity 100 louder than 127); its pairs' velocity went from 6 to 4, and it reads 1.54 |
| L11 | Level across the keys | Under 3 dB | Met on 9 of 10 (2.8 dB at the most). Init reads 3.3 dB: a lone sine, which the loudness weighting hears louder up the keys |
| B5 | Rubber Bass on small speakers | At least −12 dB | Met: −7.5 dB (−85 before: a sine by 0.1 s) |

Rows made worse, and why (each still within its target unless said): mono fold-down loss rose from 0 to at most 0.26 dB (Drawbars' rotary; the others' plates and room) since the effects are stereo; the 8-note chord's peak rose where a preset was made louder to sit level with its category (Brass Section −11.6 to −6.4 dBFS, Rubber Bass −8.5 to −6.4, Drawbars −1.26 to −1.20); the 6-note chord against its notes' power moved by up to 0.3 dB (Steel Pluck 1.57, Drawbars 1.60, Brass Section 0.38); single notes across the keys moved by up to 0.3 dB on Steel Pluck (2.64 dB: its plate, which has no wobble now) and by a few hundredths on Log Drum and Init (the room, and the DC blocker on a lone sine); Rubber Bass's first-frames step rose from −56.7 to −50.2 dBFS (it is 12 dB louder, level with the other basses); the square-law fit of level against velocity moved on five presets (Bell Tower 0.93, Brass Section 0.61, Log Drum 1.59, Rubber Bass 1.95, Steel Pluck 1.08 dB: with their DC gone, what is left is the tone's own level, and the presets brighten with velocity now); tone against velocity fell on Glass Keys from 0.37 to 0.30 octave (its pairs' velocity went from 6 to 4, for L6) and on Bell Tower from 2.17 to 2.11; FM's Bessel fit moved by 0.0015 dB and feedback 7 against 0 by 0.001 dB (the DC blocker). The plates and the room run with their Mod at 0, so a silent FM track falls to exact silence (see [Synths](synths.md#fm-mpfree-synth-fm)); Reed Lead's plate went from 10 to 7 % so that, without the wobble, its notes stay within 3 dB across the keys (2.42 dB). Each was re-blessed in `quality/synth.tsv` with the change.

## The NEON kernels (C3)

The bar for each kernel in `mpfree-simd` ([NEON kernels](simd.md)): the supersaw's saws (`table_sum`, N1 to N4), the Choir's formants (`formants4`, N5 to N7) and the divide-down bank (`waves4` and `mix4`, N8 to N10). N1, N2, N4, N5, N6 and N8 run on CI in the ARMv7 job; N9 is counted with `tools/inst_arm_cost.py`; N3, N7 and N10 are the device round's.

| Id | Measure | Target | Today |
|---|---|---|---|
| N1 | The kernel against its plain twin under qemu-arm | The same samples and phases to the bit: random calls (8 to 4,096 entries, 1 to 9 oscillators, 0 to 69 frames, writing and adding), every saw level at Poly's sizes, the bench workload, and denormals with flush-to-zero on | Met: 13 tests, 3,000 random calls |
| N2 | ARMv7 instructions a block, eight voices' supersaw saws with their mixing (`tools/simd_arm_cost.py`) | NEON fewer than the twin | Met: 58,361 against 134,455 (43 %) |
| N4 | Poly with the kernels on and off, as ARMv7 under qemu (`mpfree-synth/tests/simd.rs`) | The same samples: every Super preset, and both oscillators on Supersaw with unison | Met |
| N3 | Time on the MPC, the same case (`mpfree-bench`, "Supersaw saws: 8 voices, plain" and "..., NEON") | NEON's p99 block under the twin's, or the kernel is removed; and the Super presets no slower with the kernels on than with `--plain` | Device round |
| N5 | The formant kernel against its plain twin under qemu-arm | The same samples and state to the bit: random calls (0 to 116 frames, filters 50 Hz to 0.45 of the rate, Q 0.5 to 50, signed and -0 gains, silent lanes), a voice dying away with flush-to-zero on, and the Choir workload | Met: 3,000 random calls |
| N6 | ARMv7 instructions a block, eight Choir voices' formants with their source and mixing; and the Choir engine with the kernel on against off | NEON fewer than the twin | Met: 40,486 against 126,280 (32 %); the engine 81 to 89 % (Init 1 and 6 keys, Big Choir 8) |
| N7 | Time on the MPC, the same case ("Choir formants: 8 voices, plain" and "..., NEON"), and the Choir engine's cases with and without `mpfree-bench --plain` | NEON's p99 block under the twin's, or the kernel is removed | Device round |
| N8 | The bank's kernels against their twins, and Strings and the combo organ with the kernels on and off, as ARMv7 under qemu | The same waves, flags, samples and levels to the bit: random calls, four notes at a time against one at a time (12-TET and 24-TET), all 26 Strings presets and the 8 Combo presets | Met |
| N9 | ARMv7 instructions a block with the kernels on against off (`tools/inst_arm_cost.py`) | Fewer with 6 keys or more, no more with 1 | Met: Strings Init 6 keys 87 %, 49 keys 76 %, Orchestra 84 %, Combo 71 %, Combo Reed 73 %; 1 key flat |
| N10 | Time on the MPC, the Strings and Combo cases with and without `mpfree-bench --plain` | Faster with the kernels, or they are removed | Device round |

## Wave 2 targets: Wind (stage 4)

The bar for the [Wind engine](synths.md#wind-mpfree-synth-wind), from [Organs, winds, choirs and tape](../research/organ-wind-choir.md#4-what-to-measure) and [Wind instruments](../research/wind.md#4-what-to-measure). Its 20 presets also meet the voice core's rows (V) and the shared rows: L2 (−8.2 to −1.1 dBFS), L3 (under −82 dBFS), L5 (0.10 to 0.74 octave), L6 (the square law within 0.76 dB), L8, L11 (1.0 to 2.0 dB), L12 (Reed Section and Flute Choir), T1 (0.13 cents at worst) and A1 (9 of 9: at C7 −83 dB for the clarinet at Tone 100, −64.5 for the sax at Reed and Tone 100, −70.7 for the flute at Edge and Tone 100). Built 19 Nov.

| Id | Measure | Target | Today |
|---|---|---|---|
| O3 | Breath, at C4, from a breath controller at 1/8 to 1: the level and the brightness (the centroid) rise at every step, brightness at least 0.1 octave over the range; and the note speaks (within 3 dB of where it settles) within 30 ms at velocity 127 | No step down; 0.1 octave; 30 ms | 11 of 12. The level rises at every step on all three bores (17.2 to 17.4 dB over the range) and the brightness on the clarinet and flute (0.68 and 0.57 octave); the notes speak in 21 ms (clarinet), 25 (sax) and 30 (flute). Not met: the sax's brightness falls for two of seven steps from 1/8 to 3/8 before it rises (0.22 octave over the range): at the softest breaths the cone plays a narrow pulse whose strong second harmonic the breath first smooths. Lowering the sax's tone floor meets the row but leaves a dark, soft-reed sax silent until half breath |
| O4 | Tuning, C2 to C6 at velocities 30, 100 and 127, each bore | Within 3 cents | Met: clarinet 0.09 cents, sax 0.03, flute 0.17 |
| Cost | What a voice costs (no wind row in the survey; reported, as B11 is) | Reported | Workstation, a sixteenth-note line, instructions a block: Wind Init 121k, Growl Sax 144k, Concert Flute 143k; Reed Section, 6 voices on chords, 608k. MPC: device round |

## Wave 2 targets: Choir

The Choir's bar ([Synths](synths.md#choir-mpfree-synth-choir)); the ids are the research page's ([Organs, winds, choirs and tape](../research/organ-wind-choir.md#4-what-to-measure)). Built 19 Nov; every row below is in the report and the ratchet but O10, which is the device's.

| Id | Measure | Target | Today |
|---|---|---|---|
| O1 | Formants: each of the five filters' peak and half-power width against the vowel table, for each section at ooh, oh, aah, eh and ee (the engine's own filters, read at C2) | Peaks within 5 %, widths within 20 % | Met for all five sections: peaks within 1.5 %, widths within 7.5 %, levels within 0.07 dB |
| O2 | Vowel morph: the most energy between the harmonics of a B4, against the loudest harmonic (the B4 is tuned a little so a step at every control step, each 16 frames, would land between its harmonics), at rest, turning the Vowel knob from ooh to ee and back over 3 s, and with Morph 60 at 0.15 Hz | No step above −60 dB | Met: −139, −95 and −104 dB |
| O8 | Singers: the mean correlation of eight singers' pitch movement over 2 s with vibrato 30 cents; stereo: correlation of the Init sound holding C major over two octaves | Singers under 0.5; stereo 0.2 to 0.6 | Met: 0.45; 0.52 |
| O10 | Cost: a 6-note chord of 4 singers each | Under 50 % of the synth budget on the MPC | Device round. Workstation, instructions a block: Init (3 singers) 213k for one key, most of it the Ensemble, 359k for 6; Big Choir (8 singers) 619k for 8 keys |

On the rows every preset meets, the Choir's 25 presets: T1 0.000 cents from C1 to C7 on the plain patch; A1 −99 to −80 dB (the worst ee at C7); L2 every 8-note chord at full velocity under −3.3 dBFS; L3 DC under −104 dBFS; L5 0.11 to 1.17 octaves (Solo Voice needed Brightness 75: an alto's aah at the test note barely brightens with velocity, a tenor's moves 0.87 octave); L6 13 to 18 dB from velocity 20 to 127, within 0.7 dB of the square law; L8 under 1.6 dB; L12 within 2.6 dB on the 23 poly presets; L1 −25.9 to −22.6 LUFS (Choir Stab −28.1, short by design).

**Not yet: L11** on 6 of 25 presets: Ooh Choir 4.2 dB from C2 to C6, Aah to Ee 3.9, Choir Stab 3.8, Pressure Vowels 3.6, Oh Choir 3.5 and Bright Aahs 3.4. The engine is level across the keys (Init within 0.5 dB, Solo Voice 0.2) because each section, vowel and pitch has a trim measured from its own formants; what is left is the singers beating against each other and the Ensemble's comb on vowels with few strong harmonics, so the spread moves with the singers' random starts (2.0 to 3.1 dB with other seeds).

## Wave 3 targets: Pluck (stage 7)

The bar for [Pluck](synths.md#pluck-mpfree-synth-pluck): the shared rows every engine meets, plus the comb (U1) and Pluck's K3, set before the work and measured before (12 presets, 19 Nov, main `e8b7918d`) and after (22 presets, the same day). Research: [Plucked strings and struck bars](../research/plucks.md#4-what-to-measure). Every Pluck row is met.

| Id | Measure | Target | Before | After |
|---|---|---|---|---|
| U1 | Position's comb (above) | under −30 dB | −7.0 dB at Position 50 (even harmonics barely down); −5.5 dB for the 5th at Position 20 | −50.0 dB; −44.0 dB |
| L2 | An 8-note chord at full velocity, peak | under 0 dBFS | 6 of 12 clip, up to +3.2 dBFS (the mallets and Steel Drum) | 22 of 22, −5.7 to −0.7 dBFS |
| L1 | The phrase's loudness | shown (the engines' plucks sit at −24 to −27 LUFS) | −31.5 to −15.7 LUFS | −26.7 to −24.8 LUFS |
| L5 | Brightness, velocity 127 against 20 | at least 0.1 octave | 6 of 12 (0.00 to 0.33) | 22 of 22 (0.11 to 0.48) |
| L6 | Level from velocity 20 to 127; the square-law fit | range shown (research: 30 to 40 dB from 1 to 127, at least 18 from 20 to 127); within 2 dB | 6.4 to 9.8 dB; met | 18.1 to 24.5 dB (9.3 for Harpsichord, set to 12 dB on purpose, as the instrument barely has dynamics); met |
| L11 | Single notes C1 to C7 (C1 to C3 for a bass) | under 3 dB | 3 of 12 (up to 9.0 dB) | 22 of 22 (0.8 to 2.6 dB) |
| L12 | A 6-note chord against its notes' power sum | within 6 dB | met | met |
| K3 | T60 at C4 against Decay; the slope against −0.5 | within 20 %; within 0.15 | met (Muted Bass +14.9 % as measured then: its label is for the note it plays, an octave down) | met (Muted Bass −9.2 %; the row reads the label at the transposed pitch since 19 Nov) |
| B4 | A bass's first four frames at C2, the largest step | under −40 dBFS | met | met (−57.0 and −54.9 dBFS: a fresh string's comb starts from silence) |

What's still open: per-pluck variation. Each pluck's noise differs, so one pluck of a key can be 1 to 2.5 dB louder than the next (it was 2.5 to 5.7 dB before the low harmonics were set to their average); the lab measures one pluck per key, so L11's numbers carry that much chance. A sitar and a tanpura (a string buzzing on a curved bridge) are not built: see the research page. The note-on cost (the harmonic fix, about 16k frames of arithmetic for a low note, and each key's level worked out once) wants a device round.

## Wave 3 targets: Drums

The bar for the [Drums engine's](synths.md#drums-mpfree-synth-drums) Current voices, from [Drum synths](../research/drums.md#4-what-to-measure); the targets are the research page's proposals. Built 19 Nov. The 27 factory kits (432 pads) play the Current voices, each through its own master effects where it has them (`fxlab`'s `kit_node`, as the pads play it); every row is in the report and the ratchet but D10, which the bench measures. The first five kits' numbers are in brackets where they differ.

| Id | Measure | Target | Today |
|---|---|---|---|
| D1 | The 808 kick at Tune 0 with the circuit's sigh (3.28 semitones over 300 ms): its pitch once the 6 ms punch is over and once settled, against 58 and 48 Hz (Werner, Abel and Smith, DAFx 2014), and its time to −60 dB against the label | Within 5 % | Met: 0.02 %, 0.001 % and 1.1 % |
| D2 | The metal bank's six frequencies at Spread 50 against the cymbal paper's (205.3, 304.4, 369.6, 522.7, 540 and 800 Hz), and the cymbal's two band-pass centres at Tone 50 against 3,440 and 7,100 Hz | Within 0.5 %; within 5 % | Met: 0.001 %; 0 % |
| D3 | Velocity to tone on every pad: the smallest centroid step from velocity 20 to 127 (five steps), and the attack's rise from 40 to 127 | No step down; at least 6 dB | No step down on 427 of 432 (all 80): the misses are Lo-fi Kit's two kicks and block (−0.008 octave at most, its fewer bits) and FM Kit's FM cymbal (−0.32: at full bite its high index folds its sidebands down). The attack rises 6 dB or more on 424 (all 80): the misses (4.4 to 5.7 dB) are Hardstyle Kit's kick, distorted so hard that a soft hit is nearly as hard, the 808 and Miami Bass kits' dull hats, two dark cymbals and Industrial Kit's all-noise snare. The older row (the centroid at least 0.1 octave brighter at 127 than at 40) is met on 310 of 432 (51 of 80): the misses are the tuned drums (toms, kicks, congas, zaps, woodblocks, the 808 kick, some FM percs), whose first 93 ms is mostly the note itself, which a harder hit doesn't change, even with the new kits' Vel > tone at 60 |
| D4 | The same hit twice at velocity 127, Variation 50: the level change; on each kick, its pitch | 0.2 to 1 dB; 1 to 5 cents | Level 372 of 432 (71 of 80), pitch 27 of 39 kicks (7 of 7; 4.8 cents). The level misses are mostly noise and metal pads (claps, hats, shakers), whose loudest 20 ms already moves with the noise and the squares' starting phases (0.002 to 2.7 dB); their sound always differs, so there is no machine gun. A kick distorted hard (Hardstyle, Industrial, Techno) changes by under 0.1 dB, and four short wood, bell and conga hits in the Amapiano and Latin kits by about 2 dB. The pitch misses are kicks of 350 ms or less, whose window 0.2 to 0.4 s after the hit is their last tail, and kicks in a room or driven hard (6 to 1,027 cents: the tracker follows the tail, not a body) |
| D5 | Time to −60 dB against the Decay label, every pad | Within 10 % | 379 of 432 (all 80, at most 7 %). The misses: 28 drums sent to the kit's Delay or Reverb, whose echo is part of the hit as measured; 15 in Lo-fi Kit, whose tape's hiss and crackle never fall 60 dB; and 10 near the line (cymbals 10 to 19 % short, claps and shakers 10.5 to 12 % long) |
| D6 | A ringing 808 kick choked by a silent pad in its group: the energy above 4 kHz in the 20 ms from the choke, against the kick | No step above −60 dB | Met: −87 dB (5 ms smooth fade). Classic, with its 3 ms fade, is −68 dB |
| D7 | Each pad's level (loudest 20 ms) against its kit's kick at velocity 127; each pad's peak; every pad at once, twice (32 hits) | Within 6 dB; under 0 dBFS; reported | Level 427 of 432: all 352 of the new kits' pads (none more than 5.96 dB under its kick, none above it); 75 of 80 in the first kits. The five misses are those kits' very short or quiet hats (Boom Kit's closed and open hat, −9.4 and −6.2 dB; Tight Kit's, −9.1 and −6.3; Init Kit's closed hat, −6.1): a hat that short is a tick whose 20 ms level sits low against its peak, so raising it 3 dB would use up the headroom every other pad keeps. Peaks met on all 432 (the mid at −2.7 dBFS at most, the new kits' −3.1; either side under −3.0 at 44.1 and 48 kHz). 32 hits at once peak at +8 to +13 dBFS before the kit level (Lo-fi Kit +2.3; Vintage Rhythm and Boom Bap, through tape, −2.5 and −3.5): our reading of "no pad clips at 32 hits" is each pad's own peak, since 32 drums summed at full level can't stay under 0 dBFS and still be loud alone; the master's safety limiter catches the pile-up. A limiter in every kit would hold it at −1 dBFS for about 45k instructions a block, 2 ms of delay and a ducked cymbal tail under every kick (the owner's card, 19 Nov) |
| D8 | A hit at frame 1003 rendered in blocks of 1 to 1,000 frames | Starts on its frame; the same samples | Met: 0 frames, identical |
| D9 | The metal bank's worst alias between 20 Hz and 20 kHz at 1×, against its strongest harmonic | Under −60 dB | Met: −62.4 dB, with each square's edges smoothed over four samples and a one-pole at 10 kHz on their sum. Plain squares at 1× are at −34.7 dB, and at 4× oversampling (the paper's way) −49.8 dB |
| D10 | 32 hits sounding: every pad hit each block, so 16 hits start while the 16 before fade (bench case "Drums Init Kit, every pad each block") | Under 50 % of the synth budget (10 % of a period) | Not met. 1.40 M instructions a block (the Classic voices on the same pattern: 1.17 M); from the workstation's time and the MPC calibration, about 30 % of a period. The everyday patterns cost 7 % (Init Kit, a hit every 4 blocks) and 22 % (Boom Kit, a hit every block) more instructions than Classic. No one part dominates the voice: the next step is running four hits at once on NEON, then the device round's numbers |

## Not measured yet

Supersaw detune and phase (P2, P3), unison and table-morph steps (P4, P7), Analog's own spread at each Age (P10 measures Poly's, with the same cards and drift), sustain-pedal behaviour (K5: there is no pedal event yet), piano partials (K8), FM envelope rates and banks (F4, F6), tape (O7: no engine yet; Wind's O3 and O4 and the Choir's O1, O2 and O8 are in the report), attack noises (O9), the Choir's cost on the MPC (O10), the drums' cost on the MPC (D10: the workstation's numbers are in [Wave 3 targets](#wave-3-targets-drums)), and cost on the MPC. Each joins the report when stage 3 or 4 builds or changes what it measures.

## Running it

```sh
cd rust
cargo test --release --locked -p mpfree-fxlab --test synth -- --nocapture     # the ratchet, with the report
cargo build --release -p mpfree-fxlab && ./target/release/fxlab synth --renders --out /tmp/synth
./target/release/fxlab synth --engine Pluck --out /tmp/pluck                   # one engine's presets (and its own rows), in seconds
cargo run --release -p mpfree-bench -- --afford --filter keys                 # voice costs, Keys only
```
