<!--
covers: rust/mpfree-fxlab/src/synth.rs rust/mpfree-fxlab/tests/synth.rs rust/mpfree-fxlab/quality/synth.tsv rust/mpfree-bench/src/cases.rs
checked: 2026-11-18 dd1645f
-->
# Synth quality: the baseline, the measures and the ratchet

**Status:** measure-only baseline built 2026-11-18 (synth library stage 2). No engine changed: this page says where today's synths stand against the targets the research pages propose, and the ratchet stops any of those numbers getting worse while stages 3 and 4 improve them. Research: [What every type shares](../research/synth-topics.md) and the type pages it links. Plan: "Synth library: stock-take and research plan" (https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). **Nothing here has run on the MPC**; the cost table is the workstation's.

The method is the FX quality one ([FX quality](fx-quality.md)): each target a number with its reason, an fxlab module that measures it on signals with known answers, and a ratchet on CI. Targets in brackets on the research pages are proposals until listening and the device confirm them; here they are the bar a row is "met" against, and a row that misses one is printed, not failed.

## The report

`fxlab synth [--rate HZ] [--renders] [--out DIR]` measures every keyboard engine's factory presets (Poly, Analog, Squelch, FM, Wavetable, Keys, Drawbar, Pluck: 83 presets at the baseline; Strings' 24 joined with wave 1, Drawbar's 8 combo presets with wave 2), each engine's plain patch, Poly's voice, FM's operators and the five drum kits, and prints one row per number with its target and whether it is met, then a count by measure; it writes DIR/synth.txt. With `--renders` it also writes each preset's listening phrase (C, E and G, then a four-note chord; two octaves down for a bass) as DIR/renders/ENGINE/NN-PRESET.wav, and each kit's tour (every pad, then two bars of a beat). The report takes about 21 s in a release build on the workstation. A factory preset that brings its own effects and macros (a sound, [Presets](presets.md#factory-presets-as-files)) is measured as the pads play it, through its effects; the K rows, which measure the engine's own voicing (decay against its label, the release drop, the pickups, the tremolo), play its engine alone, since a sound's echo or reverb lengthens a decay by design.

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
| K9 | Keys presets with tremolo | The level's modulation rate against Trem rate | within 5 % | K9 |
| T1 | each engine's plain patch, and Drawbar's combo (its flute on the 8') | Pitch of C1, C2, C4, C6 and C7 (a Blackman-Harris peak refined by the phase it turns through in a second): the worst, cents | within 1 cent (the tonewheel organ, tuned by its gears, shown) | S6, B1 |
| A1 | plain patches with saw, pulse, square, drive, distortion, wavetables, FM feedback; Drawbar's combo flute, saw reed and square reed | Loudest component off the harmonics, 20 Hz to 20 kHz, against the loudest harmonic, at C5, C6 and C7 | under −60 dB | P1, F3, B10 |
| O6 | Drawbar's combo, its flute on one footage of C4 at a time | The 16', 4', 2' and 1' against the 8': the worst, cents from an exact octave | within 0.1 cent | O6 |
| B2 | Poly, legato C3 to C4, Glide 200 ms | Landing pitch; time to within 12 cents (99 % of the octave) against the label | within 2 cents; within 10 % | B2 |
| P6 | Poly's amp envelope on a sine | Attack to 99 % of full level, Decay to within 1 % of Sustain, Release to 60 dB under the held level, against the labels (since 19 Nov a time means what it says; before, 99 % of a segment and Release to 1 %) | within 10 % | P6 |
| P10 | Poly's Age on a sine, 16 A4s in turn (one a card); the voice core's drift stepped every 0.1 s for 30 minutes | The notes' tuning spread at Age 100 against what the cards and the drift say; Age 50 against 100; Age 0; the drift's rms and how far it wanders | within 1.5 cents; a ratio within 0.15 of 0.5; under 0.01 cents; within 0.5 of 2 cents; under 10 cents | P10 |
| P12 | Poly's play routes, the mod wheel at the top | A route on Pitch at 100 % against an octave up; the default route (the wheel on Vibrato at 50 %) against half a semitone each way | within 1 cent; within 5 % | P12 |
| B9, P5 | Poly's LP24 and LP12 at 1 kHz | A C1 saw's harmonics filtered against the same saw with the filter open, against the analog prototype (`1/(1+s)^4`, `1/(1+s)^2`) where it is above −30 dB; where it is 3 dB a pole down against the label; full resonance on noise, where it peaks at 250 Hz, 1 kHz and 4 kHz | within 1 dB; within 0.1 octave; within 25 cents | B9, P5 |
| F1 | FM, operator 2 at ratio 4 into operator 1 | Partials at 1 ± 4n of C4 against the Bessel functions |Jn(I)|, I fitted, at operator levels 60, 75 and 90 | within 0.5 dB | F1 |
| F5 | FM, operator 6 alone | Feedback 7 against 0, LUFS | within 3 dB | F5 |
| F7 | FM Glass Keys | LUFS spread over the 32 algorithms | under 3 dB | F7 |
| D3 | every drum pad of every kit | Centroid of the first 93 ms, velocity 127 against 40, octaves | at least 0.1 | D3 |
| D4 | every pad | The same hit twice, the second's level against the first | shown | D4 |
| D5 | every pad | Time to 60 dB under the loudest 5 ms against the Decay label | within 10 % | D5 |
| D7 | every pad | Level against the kit's first kick | shown | D7 |

New analyses in `rust/mpfree-fxlab/src/synth.rs`, each tested on signals with known answers (`cargo test -p mpfree-fxlab --lib synth`): the frequency read (a sine 0.07 % off reads within 0.01 cent at C1, C4 and C7), the alias read (a band-limited saw under −85 dB, a naive one above −40 dB), the Bessel functions and the square-law fit.

## The ratchet

`tests/synth.rs` runs the whole report and compares every row that has a better direction (1,755 of them on 19 Nov) with `rust/mpfree-fxlab/quality/synth.tsv`, as the FX ratchet does: a number worse than recorded by more than its tolerance, a new row, or a recorded row no longer measured fails it. It renders every preset many times, so it runs in release only (ignored in a debug build) as its own CI step, about 21 s. A change that improves numbers records them with `MPFREE_QUALITY_BLESS=1 cargo test --release --locked -p mpfree-fxlab --test synth`; one made worse on purpose is blessed the same way and explained in its pull request. Adding a preset adds rows, so a preset pull request blesses too.

## Where today's synths stand (2026-11-18)

Wind (19 Nov) adds 20 presets that meet every shared row (L2, L3, L5, L6, L8, L11; [Wave 2 targets](#wave-2-targets-wind-stage-4)); the counts here are from before it. Met against rows with a target (with Strings, Bass, the Keys clav, the first six factory sounds and, 19 Nov, Poly's new presets, Worn Brass among them, and Drawbar's combo): L2 163 of 180, L3 177/180, L5 108/160 (the six tine presets that missed it meet it since the tine's rebuild, 19 Nov), L6 179/180, L8 180/180, L11 130/180, L12 126/126; B1 2/2, B3 3/3, B4 50/50, B5 31/50, B7 50/50; K1 23/23, K3 92/92, K4 11/11, K7 5/5, K9 8/8; S7 8/8; V7 2/2; T1 10/10; A1 40/63; O6 1/1; B2 2/2; P6 3/3; P10 5/5 (19 Nov: 0.07 cents off its label, a ratio of 0.50, 0.0001 cents, 2.0 cents rms, 8.5 cents at most); P12 2/2 (19 Nov: 0.0001 cents, 0.06 %); B9 3/3, P5 2/2; F1 3/3, F5 0/1, F7 0/1; D3 19/80, D5 70/80.

- **In tune and true to theory.** Every engine's plain patch is within 0.03 cents from C1 to C7, except Keys (0.55 cents at C2) and the tonewheel organ (0.58 cents, its gear ratios); Drawbar's combo is within 0.0006 cents, its octaves exact (O6). FM's sidebands match the Bessel functions within 0.01 dB (operator level 60, 75 and 90 give an index of 0.43, 1.57 and 5.76). Poly's LP24 is within 0.16 dB of its 4-pole prototype and LP12 within 0.8 dB, the cutoff where the label says, the resonance peak within 5.3 cents. Glide lands within 0.04 cents in 1.2 % of its time. Keys' and Pluck's decays follow their labels and laws (all 88 K3 rows), and the tremolo rates are within 0.4 %.
- **Aliasing is the biggest gap.** Poly's saw and pulse (PolyBLEP) alias at −49 dB at C5, −44 at C6 and −37 at C7 (the saw's 13th harmonic folds to 16.9 kHz); drive and distortion make it −30 and −27 at C7. (Fixed for Poly 19 Nov, wave 2: from the voice core's tables its saw and pulse are at −78, −84 and −90 dB and its triangle −94 to −99; the antialiased drive is at −50, −45 and −38 and the distortion −45, −40 and −35, still short of −60.) Analog: −54, −48, −42. Squelch's saw meets −60 to C6, its square doesn't (−27 at C7). Wavetable's mips work: Init −72 at C7, position 100 −54. FM feedback 7 sits 13 to 17 dB under its harmonics with what is off them (aliasing or the feedback's noise; the row can't tell which).
- **Velocity is narrow.** From velocity 20 to 127 Poly's level moves 7 to 8 dB, Analog's 2 to 8, Pluck's 6 to 10 and Keys' 12 to 22, against the 30 to 40 dB the research proposes; the curve's shape is fine (81 of 83 within 2 dB of a square law). Tone follows velocity on 32 of 71 presets; on the drums only snares, claps and FM perc brighten (19 of 80 pads).
- **Levels.** 17 presets clip an 8-note chord at full velocity (up to +3.2 dB: Pluck's mallets and Steel Drum, Poly Init +2.0, FM Init +1.9). Single notes across the keys stay within 3 dB on only 25 of 83 presets (Pluck's strings up to 9 dB). DC is above −60 dBFS on 8 presets: FM's 1:1 pairs (Glass Keys −38.5 dBFS: a 1:1 pair puts a sideband at 0 Hz), the reeds (Reed Bite −45.6) and Poly Lead (−45.1). Every preset keeps its loudness within 3 dB in mono.
- **Knobs that say one thing and do another.** Poly's Attack reaches full level in 31 % of its label (the envelope aims at 1.3 and stops at 1), and Release reaches 1 % in 85 % of its label (it aims just below zero); Decay is exact. FM feedback 7 is 3.5 dB louder than 0, and Glass Keys moves 6.4 dB across the algorithms.
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

## The NEON kernels (C3)

The bar for each kernel in `mpfree-simd` ([NEON kernels](simd.md)): the supersaw's saws (`table_sum`, N1 to N4), the Choir's formants (`formants4`, N5 to N7) and the divide-down bank (`waves4` and `mix4`, N8 to N10). N1, N2, N4, N5, N6 and N8 run on CI in the ARMv7 job; N9 is counted with `tools/inst_arm_cost.py`; N3, N7 and N10 are the device round's.

| Id | Measure | Target | Today |
|---|---|---|---|
| N1 | The kernel against its plain twin under qemu-arm | The same samples and phases to the bit: random calls (8 to 4,096 entries, 1 to 9 oscillators, 0 to 69 frames, writing and adding), every saw level at Poly's sizes, the bench workload, and denormals with flush-to-zero on | Met: 13 tests, 3,000 random calls |
| N2 | ARMv7 instructions a block, eight voices' supersaw saws with their mixing (`tools/simd_arm_cost.py`) | NEON fewer than the twin | Met: 58,361 against 134,455 (43 %) |
| N4 | Poly with the kernels on and off, as ARMv7 under qemu (`mpfree-synth/tests/simd.rs`) | The same samples: every Super preset, and both oscillators on Supersaw with unison | Met |
| N3 | Time on the MPC, the same case (`mpfree-bench`, "Supersaw saws: 8 voices, plain" and "..., NEON") | NEON's p99 block under the twin's, or the kernel is removed; and the Super presets no slower with the kernels on than with `--plain` | Device round |
| N5 | The formant kernel against its plain twin under qemu-arm | The same samples and state to the bit: random calls (0 to 116 frames, filters 50 Hz to 0.45 of the rate, Q 0.5 to 50, signed and -0 gains, silent lanes), a voice dying away with flush-to-zero on, and the Choir workload | Met: 3,000 random calls |
| N6 | ARMv7 instructions a block, eight Choir voices' formants with their source and mixing | NEON fewer than the twin | Met: 40,486 against 126,280 (32 %) |
| N7 | Time on the MPC, the same case ("Choir formants: 8 voices, plain" and "..., NEON") | NEON's p99 block under the twin's, or the kernel is removed | Device round |
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

## Not measured yet

Supersaw detune and phase (P2, P3), unison and table-morph steps (P4, P7), Analog's own spread at each Age (P10 measures Poly's, with the same cards and drift), the 808 kick and metal bank against their papers (D1, D2, D9), chokes and retriggers (D6), sustain-pedal behaviour (K5: there is no pedal event yet), piano partials (K8), FM envelope rates and banks (F4, F6), choirs and tape (O1, O2, O7, O8 and O10: no engines yet; Wind's O3 and O4 are in the report), attack noises (O9), and cost on the MPC. Each joins the report when stage 3 or 4 builds or changes what it measures.

## Running it

```sh
cd rust
cargo test --release --locked -p mpfree-fxlab --test synth -- --nocapture     # the ratchet, with the report
cargo build --release -p mpfree-fxlab && ./target/release/fxlab synth --renders --out /tmp/synth
cargo run --release -p mpfree-bench -- --afford --filter keys                 # voice costs, Keys only
```
