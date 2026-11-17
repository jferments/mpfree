<!--
covers: rust/mpfree-fx/src/modfx.rs
checked: 2026-11-17 68709b4
-->
# Modulation: Chorus, Flanger and Phaser

**Status:** built (F2); FX quality round 3 is under way. The research, targets and baseline are done (61 of 113 measured targets met on `main`). The chorus's rebuild, PR M1 (with the fxlab `modulation` class, the synced LFO's fix and a flat read), is written and measured but not yet open; its design is described below as "M1" and is not on `main` until that PR merges. The flanger (M2) and phaser (M3) follow, each its own pull request on Jay's go.

This page is the design of MPFree's three modulation effects as one effect type: what they are for, the ideas underneath them, the research behind ours, which algorithms each uses and why, how quality and cost are measured, and how the defaults and presets were chosen. The parameter-by-parameter summary lives in [fx.md](fx.md); the measured targets and their history in [fx-quality.md](fx-quality.md). The research notes, plan and baseline probe are in jferments/mpfree-claude `fx/quality/` (research-modulation.md, plan-3.md, tools/mod_baseline.rs, tools/mod_stereo_proto.py).

| Effect | Node id | Code | One line |
|---|---|---|---|
| Chorus | `mpfree.fx.chorus` | [`modfx.rs`](../../../rust/mpfree-fx/src/modfx.rs) | Up to four voices of a delay of 2 to 40 ms swept slowly: thickening, width, ensemble |
| Flanger | `mpfree.fx.flanger` | `modfx.rs` | A delay of 0.1 to 10 ms swept in octaves, with feedback: the jet sweep and metallic combs |
| Phaser | `mpfree.fx.phaser` | `modfx.rs` | 2 to 12 all-pass stages swept around a centre, with feedback: the swirl |

## 1. What they are for

All three mix a sound with a copy of itself whose timing or phase moves slowly; what moves, and by how much, makes the difference (research-modulation.md section 1).

| Effect | What moves | Typical settings | Jobs |
|---|---|---|---|
| Chorus | A delay of 5 to 30 ms, swept by a few ms | Rate 0.1 to 3 Hz, Mix about 50 % | Thicken a pad or a clean guitar, widen a mono source, an ensemble of slightly detuned copies; fast and shallow for shimmer; wet only for vibrato |
| Flanger | A delay of about 0 to 10 ms, swept widely, with feedback | Rate 0.05 to 1 Hz, feedback of either polarity | The jet sweep, metallic resonance on drums and guitars, a held comb as a resonant filter, tape flange on a whole mix |
| Phaser | The corner of a chain of all-pass filters, swept in octaves | 4 stages (two notches), Rate 0.1 to 5 Hz, some feedback | The swirl on guitars, keys and pads, funk wobble, phased drums |

The familiar units set what musicians expect: the Boss CE-2 (a BBD delay swinging between about 8.5 and 10.75 ms, a triangle LFO up to about 3.5 Hz, 50/50 dry and wet, a passband to about 6.6 kHz), the Juno-60's chorus (0.5 and 0.83 Hz triangles, the two channels 180° apart), the Dimension D (opposite sweeps and cross-fed sides, width without obvious wobble), the Eventide Instant Flanger (two BBDs, in-phase or out-of-phase mixing), and the Phase 90 and Small Stone phasers (four stages, two notches; the later Phase 90 with feedback).

## 2. Principles

**A moving delay is a pitch bend.** Reading a delay whose length changes by `d'` seconds per second plays the copy at a pitch ratio of `1 - d'`. A sine sweep of ±`s` seconds at `f` Hz has `d'` up to `2π f s`, so the peak bend is about `2π f s` as a fraction (the Doppler result). Listeners notice slow frequency modulation of about ±0.35 % (6 cents) near 4 Hz (Zwicker and Fastl), so a chorus's ±5 to 30 cents is a gentle, audible detune; the CE-2 at full rate reaches about ±27 cents. Past a semitone a chorus is "seasick", and if `d'` passes 1 the read runs backwards. So depth and rate together decide the wobble, and a well-behaved chorus bounds their product.

**Mixing a copy makes a comb.** Dry plus a copy delayed by `d` cancels at odd multiples of `1/(2d)`: harmonically spaced notches. For a flanger (`d` of 0.1 to 10 ms) the notches are spread across the band and sweeping them is the effect; equal dry and wet makes them complete (Dattorro: an equal blend for "maximum trough depth"). For a chorus (`d` of 10 ms and more) the first notch is under 50 Hz and the rest are dense and moving, so above the bass the wet copy is effectively a different moment of the music: uncorrelated with the dry, like a delay's echo. That is why a linear 50/50 chorus loses about 3 dB (two uncorrelated halves at -6 dB each), and why the chorus takes the delays' equal-power Mix.

**A phaser's notches are not harmonic.** N first-order all-passes with corner `f_c` turn the phase by up to N·180°; summed with the dry, the result cancels where the chain's phase is an odd multiple of 180°, at `f_c · tan((2k + 1) · 45° / N)`. Four stages give two notches, at 0.41 and 2.41 times `f_c`. Sweeping `f_c` in octaves moves them evenly to the ear.

**Feedback adds resonance and level.** A comb or all-pass chain with feedback `g` raises its peaks toward `1/(1 - |g|)` and its noise power by `1/(1 - g²)`, +10 dB at 95 %. Positive feedback sharpens the peaks between the notches (metallic), negative shifts them (hollow). It should ring, never run away, and the level should not jump as it turns up. Feeding back a modulated read adds the pitch change on every trip, which is the sound of a flanger but limits how far feedback and fast sweeps can go together (Dattorro).

**Interpolation must not colour the copy.** A delay of a fraction of a frame needs interpolation. Linear interpolation is a low-pass that changes with the fraction; a 4-point cubic droops at half a frame (-3.4 dB at 15 kHz at 48 kHz), so as the sweep crosses fractions the highs flutter and a bright tone grows sidebands. Inside a flanger's loop the loss compounds every trip, and its high notches fill in. Dattorro calls an interpolator that does not low-pass the copy "critical" for chorus and flanger. The still-read trick the delays use (a first-order all-pass, flat in magnitude) does not work here, because its state is only right for a fraction that stands still; a moving read needs a flat FIR interpolator, a windowed sinc (Laakso et al.).

**Stereo comes from the sides sweeping differently.** Each side's copies swing on LFO phases offset from the other's: a quarter cycle (Dattorro's quadrature) or half (the Juno, the Dimension D). The offsets must leave the two sides different at every voice count, and the mono fold must not hollow out: a wide effect that vanishes in mono is a mix problem.

**A synced LFO follows the song, not the clock.** A sweep synced to a bar should stay on the bar when a loop wraps or the song jumps. It should get back by gliding, since a step in a delay is a click.

## 3. Research

Research for this class (17 Nov 2026, research-modulation.md) covered the design literature (Dattorro's "Effect design, part 2"; Laakso et al. on fractional delays; Zwicker and Fastl and Moore and Sek on FM detection), circuit analyses of the classic units (ElectroSmash on the CE-2, Electric Druid on BBD choruses, Tylman on the Juno-60, a Sound On Sound review of the Dimension D), manufacturer and designer notes (Eventide on flanging; the PhaseMistress and Fix Phaser manuals; Sound On Sound's "How phasers work"; Premier Guitar), and a baseline of MPFree's own three effects measured by a probe (`tools/mod_baseline.rs`). Findings that shaped the design:

- **Settings:** chorus delays 1 to 30 ms (Dattorro: 5 ms nominal, the CE-2 about 9 to 11 ms), flange 0 to 10 ms (the "strong flange zone" the first 1 ms), phasers 2 to 12 stages with 4 the classic.
- **What can be measured:** insert level (BS.1770), level across Feedback, wet left/right correlation and mono fold, the peak bend of a 1 kHz tone, held flatness at whole and half frames, sidebands of a moving read, notch positions and depths against the drawing, the range a flanger sweeps, synced phase after a loop wrap, feedback stability, clicks while controls move. That became targets M1 to M14 (section 7).
- **Familiar features:** Rate, Depth, Delay (Manual), Voices, Width, Tone, Feedback either way, Stages, tempo Sync; then a triangle LFO, dotted and triplet syncs, through-zero flanging, a vintage BBD colour, phaser stereo modes.
- **Where MPFree stood** (the baseline): the chorus made a track 3.3 to 3.7 LU quieter on insert (a linear 50 % Mix of an uncorrelated copy); with 4 voices its two sides were identical (correlation 1.000, so "Lush ensemble" had no width) and with 3 nearly so; its sweep was a share of the Delay, so a longer Delay was 8 times more pitchy and the deepest settings bent 2 to 6 semitones, at worst reading backwards; its cubic read lost 3.4 dB at 15 kHz at half a frame. The flanger's read dulled every trip, so its high notches were 24 dB deep instead of 80 and its drawn comb was up to 32 dB wrong; its sweep reached 40 ms. Every synced LFO fell half a cycle off the bar when a 3.5-bar loop wrapped. The phaser's notches were exactly where drawn. Feedback was stable everywhere tried.

The conclusion: the three keep their structures (a swept delay, a swept delay with feedback, a swept all-pass chain), and the work is a flat read, Depth that means milliseconds, width at every voice count, level, sync and the flanger's range.

## 4. The engines

### The shared LFO

One LFO type drives all three: a sine from a parabola with a cubic correction (error under 0.1 %, plain arithmetic, so a render is the same on every machine), its phase in f64 so a long free run never drifts. It runs free (Rate in Hz) or synced to the tempo, from 1/16 to 8 bars a cycle; **Width** offsets the right side's phase, so 100 % is a wide stereo sweep and 0 keeps both sides together. On `main` a synced LFO is set from the song position only when the transport starts.

From M1: while the transport plays, a synced LFO compares itself with the song position at the start of every block. A jump (a loop wrap, a locate) is caught up by adding a little phase each frame, so the swept delay glides instead of stepping, over 50 ms or, for the chorus, as slowly as it takes to add no more than the pitch ceiling's semitone of bend (a full-depth sweep half a cycle off takes about 0.3 s; at most a second); a jump during a catch-up restarts it with the whole gap. Catching up fast would chirp: half a cycle in 50 ms at a 5 ms sweep is a bend of several semitones. Small differences from rounding between the tempo and the frame count are snapped away when no catch-up is running.

### Chorus

**On `main`:** two lines (one a side) read with a 4-point cubic (Catmull-Rom). Up to four voices read each line at LFO phases 0, ½, ¼ and ¾ (any count starts from the same phases, so adding a voice never moves the others), with gains of 1/√n so the wet level holds as voices are added; the right side's voices are a quarter cycle on at full Width. The sweep is ±(Depth/2) of the Delay. **Tone** is a one-pole low-pass on the wet signal, keeping it warm as a BBD's was. Mix is linear; every knob glides over 30 ms and a voice-count change crossfades over 20 ms.

**From M1**, the chorus changes in four ways:

1. **A flat read** (`FlatLine`). Each voice reads through an 8-tap windowed sinc: a Kaiser window (β 4.0) on a sinc with its cutoff at 0.47 cycles a frame (20.7 kHz at 44.1 kHz), its taps normalised to sum to 1 so the lows pass exactly. The window is the Kaiser curve less its value at the edges (`I0(...) - 1`), so the outermost taps go to zero smoothly: with a plain Kaiser window the taps jumped as the read crossed a frame, which clicked (the click measure, M13, read -59 dB and a 12 kHz tone's sidebands -67 dB; with the edge-zero window the sidebands are -87 dB and the clicks are gone). The taps for 64 fractions of a frame are built at compile time (`const fn`s for sin, the Bessel function and the square root), so the table is the same on every machine, and a read between two fractions blends their rows. The line writes every sample twice, a line's length apart, so the eight taps of any read sit side by side in memory with no wrap test, and the taps are summed as four lanes of two. Every fraction sounds the same within 0.16 dB to 16 kHz at 44.1 kHz, so a sweep never flutters the highs. The read tilts the very top a little, the same at any fraction: 0.3 dB down at 14 kHz and about 1 dB at 16 kHz (the cubic lost up to 3.4 dB at 15 kHz at half a frame, and 0.9 at a whole one). The cutoff and window were chosen for that sameness: a higher cutoff droops less at 16 kHz but varies more between fractions (a cutoff of 0.48 with β 3: 0.6 dB down at 16 kHz, 0.26 dB apart), and longer reads (10 to 16 taps) were flatter in a prototype but cost more. The read sits between the 4th and 5th tap, so the shortest delay is 4 frames.
2. **Depth in milliseconds.** Depth 100 % is a sweep of ±5 ms (the CE-2 swings about ±1.1 ms, Dattorro's white chorus about ±4), never more than 90 % of the Delay, and never more than about a semitone of bend at the Rate set: the sweep in frames is at most `0.055 / (2π · inc)`, with `inc` the LFO's cycles a frame (a semitone down is a ratio of 1 - 2^(-1/12) = 0.0561, less a little for the LFO's shape). One glided value (`sweep`, 30 ms) aims at the smallest of the three; two separate glides with a minimum between them kinked as Rate swept, which clicked. So the Delay moves where the copies sit and not how much they wobble, and however fast the Rate the bend stays inside a semitone.
3. **Width at every voice count.** The left keeps the voice phases 0, ½, ¼, ¾; the right side's voices sit between the left's, a quarter cycle on for one or two voices and an eighth for three or four, times Width. With three voices at a quarter, or four, the right's set of phases was the left's (the right of voice 1 is the left of voice 3), so the sides matched. The offset glides over 250 ms when the voice count changes, slow enough that the pitch barely bends, and the voices' gains over 20 ms. A numpy model (`tools/mod_stereo_proto.py`) chose the offsets: every count ends within ±0.08 correlation above 300 Hz, for 0.2 to 0.4 dB more lost in mono.
4. **The Mix curve.** The chorus joins the delays' and reverbs' Equal, Dry first and Linear choice (`kit::Blend`, 30 ms), Equal power by default (Jay, 17 Nov). At 50 % dry and wet both sit at -3 dB, so the comb's notches are as deep as before and the level holds.

The chorus also gets a Main page (Rate, Depth, Delay, Mix on the knobs), since it has nine controls with the Mix curve.

### Flanger

On `main`: one line a side, read with the cubic, swept in octaves around **Delay** (0.1 to 10 ms), ±2 octaves at full **Depth**, so the comb moves evenly to the ear. **Feedback** (-95 to +95 %) takes the read back into the line: positive is metallic, negative hollow. Width offsets the right side's LFO; Mix is linear, since 50 % is the deepest notch. `Flanger::response_db` draws the comb for the page at any point of the sweep.

What M2 will change (plan-3 section 5): read through the flat sinc inside the loop, so the high notches stay deep and the drawn comb is right (the cubic in the loop dulls every trip, which is the 24 dB notches and the 32 dB drawing error); clamp the sweep so its longest delay is 20 ms (past that it is a doubler, not a flange); a fixed output trim so the defaults sit within 1 LU; and a feedback gain that follows the loop's noise gain (about √(1 - g²) on the wet) so more feedback sharpens the resonance without shouting. Through-zero flanging, the tape flange in which the copy passes the dry and cancels completely, is a decision for Jay when M2 comes up: it means delaying the dry by the sweep's centre, a few milliseconds of latency.

### Phaser

On `main`: 2, 4, 6, 8 or 12 first-order all-pass stages (**Stages**), each a zero-delay (trapezoidal) design, so a fast sweep stays clean, swept ±2.5 octaves around **Centre** (100 Hz to 8 kHz) at full Depth. Coefficients are worked out every 16 frames (every 0.36 ms at 44.1 kHz, far quicker than the sweep) with a straight glide between, a sixteenth of the cost of every frame. **Feedback** (-95 to +95 %) takes the chain's output back into its input one frame late, and the drawing (`Phaser::response_db`) includes that frame, which is why the drawn and heard notches match to 0.00 dB. A change of Stages crossfades over 20 ms.

What M3 will change: a fixed trim and a feedback gain, as the flanger's (the phaser is 0.6 to 1.7 LU quieter on insert and up to 7.4 LU louder at +95 %), and a stereo choice reaching opposite sweeps (180°), as the Bi-Phase and Fix Phaser's stereo modes do, appended so saved projects keep their quarter cycle.

## 5. Controls

| Effect | Controls (range, default) |
|---|---|
| Chorus | Rate (0.05 to 10 Hz, 0.6), Sync (Free, 1/16 to 8 bars), Depth (0 to 100 %, 40), Delay (2 to 40 ms, 12), Voices (1 to 4, 2), Width (0 to 100 %, 100), Tone (1 to 20 kHz, 9 kHz), Mix (0 to 100 %, 50); from M1, Mix curve (Equal) |
| Flanger | Rate (0.02 to 10 Hz, 0.2), Sync, Depth (0 to 100 %, 60), Delay (0.1 to 10 ms, 1.5), Feedback (-95 to +95 %, +50), Width (0 to 100 %, 100), Mix (0 to 100 %, 50) |
| Phaser | Rate (0.02 to 10 Hz, 0.3), Sync, Depth (0 to 100 %, 70), Centre (100 Hz to 8 kHz, 700 Hz), Stages (2, 4, 6, 8, 12; 4), Feedback (-95 to +95 %, +30), Width (0 to 100 %, 100), Mix (0 to 100 %, 50) |

What the shared names mean:

- **Rate** and **Sync**: a synced LFO ignores Rate; the sync list runs from 1/16 to 8 bars a cycle (dotted and triplet values are a planned addition).
- **Depth** on the chorus is, from M1, a sweep in milliseconds (100 % is ±5 ms, bounded as section 4 says); on the flanger and phaser it is octaves of sweep (±2 and ±2.5 at 100 %).
- **Delay** on the chorus is where the copies sit; on the flanger it is the centre of the sweep (a pedal's Manual).
- **Width** is the right side's LFO offset (0 is mono movement, not a mono output).
- **Mix**: on the chorus, from M1, a Mix curve too (Equal by default); the flanger and phaser stay linear, since their wet is close to the dry and 50 % is their deepest notch.

## 6. Defaults and presets

**Defaults** were set when the effects were built (F2) from the classic units and the literature, and kept by round 3 so saved projects keep their settings; where M1 changes what a value means (Depth), the presets are retuned to keep their sound.

| Effect | Default | Why |
|---|---|---|
| Chorus | Delay 12 ms, Rate 0.6 Hz, Depth 40 %, 2 voices | Inside Dattorro's chorus range and near the CE-2's 9 to 11 ms; a Juno-like slow rate. On `main` Depth 40 % at 12 ms is ±2.4 ms, about ±16 cents on a held note; from M1 it is ±2 ms (40 % of 5), about ±13 cents: a gentle, audible detune |
| Chorus | Tone 9 kHz | A warm wet signal, after the BBD's band limit (about 6.6 kHz in the CE-2), without dulling a mix |
| Chorus | Mix 50 %, Equal from M1 | The classic 50/50 (the CE-2); Equal power so it doesn't go in 3 dB quieter (Jay's pick, 17 Nov) |
| Flanger | Delay 1.5 ms, Depth 60 %, Rate 0.2 Hz, Feedback +50 % | In the strong flange zone, a slow jet sweep with clear resonance |
| Phaser | 4 stages, Centre 700 Hz, Rate 0.3 Hz, Feedback +30 % | The Phase 90 / Small Stone shape: two notches (held, 290 and 1684 Hz without feedback, 292 and 1740 Hz with it); a little feedback for character |
| All | Width 100 % | A stereo sweep is what these effects are for; Width 0 is there for mono sources that must stay centred |

**Presets** (a preset never sets Mix, Mix curve or Quality):

| Effect | Preset | Values | Use |
|---|---|---|---|
| Chorus | Subtle widen | Rate 0.25 Hz, Depth 15 % (M1: 12 %), Delay 8 ms, 2 voices, Tone 12 kHz | Light width on a mono source |
| | Lush ensemble | Rate 0.35 Hz, Depth 60 % (M1: 100 %), Delay 18 ms, 4 voices, Tone 8 kHz | A string-machine ensemble |
| | Dimension | Rate 0.8 Hz, Depth 25 % (M1: 15 %), Delay 6 ms, 2 voices | Short, subtle, Dimension D-like width |
| | Fast wobble | Rate 5 Hz, Depth 50 % (M1: 20 %), Delay 4 ms, 1 voice, Width 0, Tone 12 kHz | Vibrato-like shimmer |
| | Bar swirl | Sync 1 bar, Depth 55 % (M1: 77 %), Delay 14 ms, 3 voices | A swirl locked to the bar |
| Flanger | Gentle flange | Rate 0.2 Hz, Depth 50 %, Delay 1.5 ms, Feedback +20 % | A soft sweep |
| | Jet sweep | Rate 0.1 Hz, Depth 80 %, Delay 2.5 ms, Feedback +75 % | The jet plane |
| | Metallic | Rate 0.3 Hz, Depth 40 %, Delay 0.8 ms, Feedback +80 % | A ringing comb |
| | Hollow flange | Rate 0.25 Hz, Depth 70 %, Delay 1.2 ms, Feedback -70 % | Negative feedback's hollow tone |
| | Two-bar sweep | Sync 2 bars, Depth 75 %, Delay 2 ms, Feedback +60 % | A sweep locked to two bars |
| Phaser | Slow swirl | Rate 0.15 Hz, Depth 80 %, Centre 800 Hz, 4 stages, Feedback +20 % | A gentle swirl |
| | Deep phase | Rate 0.25 Hz, Depth 90 %, Centre 600 Hz, 8 stages, Feedback +60 % | Four deep notches |
| | Funk wobble | Rate 2.5 Hz, Depth 60 %, Centre 1.2 kHz, 4 stages, Feedback +40 % | A fast funk wobble |
| | Liquid | Rate 0.1 Hz, Depth 85 %, Centre 500 Hz, 12 stages, Feedback +70 % | Six notches, very resonant |
| | Quarter pulse | Sync 1/4, Depth 70 %, Centre 900 Hz, 6 stages, Feedback +50 % | A pulse on every beat |

The chorus presets' M1 Depth values keep each preset's sweep in milliseconds as it was: on `main` a preset's sweep was ±(Depth/2) of its Delay (Subtle widen ±0.6 ms, Lush ensemble ±5.4, Dimension ±0.75, Fast wobble ±1.0, Bar swirl ±3.85), and the new values give the same sweep (Lush ensemble's ±5.4 meets the ±5 ms ceiling). Saved choruses with their own settings change with the new meaning: a long Delay gets less pitchy, a short one a little more; the PR lists it.

## 7. Measuring quality

`fxlab quality --class modulation` (from M1; `rust/mpfree-fxlab/src/quality/modulation.rs`) measures each effect at its defaults and every preset, at 44.1 kHz, 113 numbers in all, with the ratchet (`rust/mpfree-fxlab/quality/ratchet.tsv`) failing a pull request that makes any of them worse. Its first commit records `main`'s numbers, so each later change shows as a diff. The targets (plan-3 section 3):

| Id | What | Target | `main` (fxlab) | With M1 |
|---|---|---|---|---|
| M1 | Insert level, pink and hits | Within ±1 LU | Chorus -3.0 to -3.7; flanger -0.3 to -2.9; phaser -0.9 to -1.9 | Chorus -0.66 to +0.06 |
| M2 | Level across Feedback (flanger, phaser) | Within ±3 LU of Feedback 0 | Flanger +6.5 at -95 %; phaser +7.4 at +95 % | M2, M3 |
| M3 | Wet left/right correlation above 300 Hz, every voice count | ≤ 0.3 | 3 voices 0.67, 4 voices 1.00 | -0.08 to 0.03 |
| M4 | Mono fold-down beyond the stereo change | ≤ 1.5 LU | Flanger "Jet sweep" 1.75, "Hollow flange" 1.54 | Chorus ≤ 1.01 |
| M5 | A 1 kHz tone's peak bend | ≤ ±100 cents, never backwards | ±866 cents at Rate 10, Depth 100, Delay 40 | ≤ ±96 |
| M6 | Bend across Delay 5 to 40 ms at one Depth | Changes ≤ 10 % | 7.9 times | 1.00 |
| M7 | Held flatness at half a frame | Within 0.5 dB to 16 kHz | Chorus 4.6 dB; flanger's high notches -21 dB | Chorus 0.16 dB |
| M8 | A 12 kHz tone's sidebands through the moving read | ≤ -70 dB | -49.8 dB | -87.3 dB |
| M9 | Drawn equals heard (flanger, phaser) | Within 0.5 dB | Phaser 0.00; flanger up to 46 dB at half a frame and -90 % | M2 |
| M10 | The flanger's longest delay | ≤ 20 ms | 40 ms | M2 |
| M11 | A synced LFO's phase after a 3.5-bar loop wraps | Within 1 % of a cycle | Half a cycle | 0.000 |
| M12 | Feedback at ±95 % with extreme settings | Finite, rings out within the tail | Met | Met |
| M13 | Clicks while a control sweeps | ≤ -60 dB | Flanger "Jet sweep" -54 dB | Chorus ≤ -66.4 dB |
| M14 | Cost | The instruction gate | Section 8 | Section 8 |

With M1 as it stands: 82 of 113 met (61 on `main`); every chorus row met; what is left is the flanger's and phaser's (M2 and M3). The numbers may still move a little before the PR opens.

How the less obvious measures work:

- **Width (M3)** is the correlation of the wet left and right on a mono pink source, both high-passed at 300 Hz, since below that every chorus's copies are close in phase whatever their layout (full band reported too).
- **Bend (M5, M6)** tracks the instantaneous frequency of a 1 kHz tone through the wet signal; M6 compares the bend at Delay 5 and 40 ms at the same Depth and Rate.
- **Flatness and sidebands (M7, M8)**: the held wet's response at whole and half-frame delays up to 16 kHz (Tone open), and the energy beyond ±5 % of a 12 kHz tone through the default chorus with one voice, wet only.
- **Clicks (M13)**: each control is swept from end to end and back, 50 ms each way, under a 997 Hz tone at -12 dBFS, and the loudest millisecond above 6 kHz while it moves is reported against the tone. A swept delay bends the tone and a resonance raises it, but neither puts energy two octaves up; a stepped control or a click does. (fxlab's usual zipper measure read a resonant comb passing the tone as a zip, 6 to 21 dB on the flanger's Delay, which is why this one replaced it.) A fast Delay sweep through a feedback loop bends the pitch on every trip, so the Delay's row is reported, not held to the target.
- **Sync (M11)**: a synced chorus is played through a 3.5-bar loop (not a whole number of cycles) and the LFO's phase just after the wrap is compared with the song's.

**Unit tests** (`tests/modfx.rs`): on `main`, Mix 0 is the input bit for bit; the chorus delays by its Delay and Width 0 is mono; the flanger's feedback makes the comb it should; the phaser notches where the stages turn half a cycle; voice and stage changes don't click; the tail estimate covers the ring-out; and the registry-wide gates (block size, extremes, NaN, allocation). M1 adds, being written now: width at every voice count; Depth in milliseconds and the semitone ceiling; a synced LFO back on the bar after a loop wrap, matching a chorus started fresh at that point; a catch-up held to the rate it is given; and the flat read flat, continuous across frames and exact at DC.

**Listening**: each PR has a blind, loudness-matched A/B page (before against after) for Jay.

## 8. Cost

The instruction gate (`mpfree-bench`, a 128-frame stereo block at 44.1 kHz, `python3 tools/ci/instr_gate.py`, on CI):

| Case | `main` | With M1 |
|---|---|---|
| Chorus, 2 voices | 95.0k | 103.8k (+9.2 %) |
| Chorus, 4 voices | 154.9k | 173.9k (+12.3 %) |
| Flanger | 55.1k | unchanged |
| Phaser, 4 stages | 61.6k | unchanged |
| Phaser, 12 stages | 100.5k | unchanged |

The rise is the read: 8 taps a voice a side where the cubic took 4. A first sinc read gathered its taps with a wrap mask for each and cost 53 to 61 % more; writing every sample twice (so the taps are contiguous), summing four lanes and skipping the glides that have settled brought it to 9 to 12 %. A four-voice chorus then costs a little more than the rebuilt Room (146k), and these effects are stacked often, which is why the read is no longer than it needs to be.

## 9. Pitfalls and how the design avoids them

| Pitfall (research section 4) | Answer | Checked by |
|---|---|---|
| Depth times rate warbles; a chorus seasick or reading backwards | Depth in ms, capped at 90 % of Delay and by the semitone ceiling at the Rate set (M1) | M5, M6 |
| Width that does nothing at some voice counts | The right's voices between the left's at every count (M1) | M3 |
| Width that vanishes in mono | Offsets chosen in a model for both width and fold | M3, M4 |
| Going in quieter | Equal-power Mix on the chorus (M1); trims on the flanger and phaser (M2, M3) | M1 |
| Feedback raising the level, extreme resonance | A feedback gain that follows the loop's noise gain (M2, M3); feedback capped at ±95 % | M2, M12 |
| Interpolation dullness, fluttering highs, shallow high notches | The 8-tap sinc read (chorus in M1, flanger's loop in M2) | M7, M8, M9 |
| A drawn comb that isn't what plays | Drawings from the effects' own response functions, the read included | M9 |
| A flanger that becomes a slapback | Sweep clamped to 20 ms (M2) | M10 |
| Sync drift after a loop wrap | The LFO follows the song every block and glides back, no faster than a semitone's extra bend on the chorus (M1) | M11 |
| Clicks as controls move | Every control glides; one glided sweep, not a kinked minimum; edge-zero sinc window | M13 |
| Fast, deep phasing out of tune | Inherent; the defaults are slow | M5 (reported) |
| Width and Manual interacting confusingly | Depth and Delay keep their own meanings; the page draws the sweep | Listening |

## 10. History and next

- **Built** (F2): the chorus, flanger and phaser with the shared LFO, tempo sync, gliding controls and their pages' drawings; factory presets for each (the FX skill tree's last nodes, 11 Nov).
- **17 Nov:** research, the targets M1 to M14 and the baseline (plan-3.md; 61 of 113 met when measured by fxlab). Jay's decision on the chorus's Mix law: Equal power by default; the flanger and phaser stay linear with a trim.
- **M1** (chorus, built, not yet open): the fxlab `modulation` class, the LFO's sync fix for all three, the flat read, Depth in milliseconds with the semitone ceiling, width at every voice count, the Mix curve, a Main page, and the chorus presets retuned to keep their sweeps. It needs Jay's go after its last code change.
- **Next:** M2, the flanger (the flat read in its loop, its range, level and feedback gain, its drawn comb; through-zero if Jay wants it); M3, the phaser (level, feedback gain, a stereo choice). Then the familiar extras (plan-3 P2): an LFO shape (sine or triangle, with a rounded corner), dotted and triplet syncs, a Vintage colour for the chorus (a BBD's band limit, emphasis, soft clip and hiss), and the phaser's stereo modes. The MPC's own chorus, flanger and phaser are not yet captured as references (FXProbe).

## 11. Sources

- J. Dattorro, "Effect design, part 2: delay-line modulation and chorus", JAES 45(10), 1997.
- T. Laakso, V. Välimäki, M. Karjalainen and U. Laine, "Splitting the unit delay: tools for fractional delay filter design", IEEE Signal Processing Magazine, 1996.
- E. Zwicker and H. Fastl, *Psychoacoustics: facts and models*, chapter 7 (just-noticeable frequency modulation); B. Moore and A. Sek, detection thresholds for sinusoidal FM, JASA 1989.
- ElectroSmash, "Boss CE-2 analysis"; Electric Druid, "Investigations into what a BBD chorus unit really does"; R. Tylman, "Understanding the Juno-60 chorus"; Sound On Sound archive, Roland Dimension D review.
- Eventide, "The flanging effect"; Premier Guitar, "Get more out of your flanger".
- Sound On Sound, "How phasers work"; vectree, "MXR Phase 90"; the Soundtoys PhaseMistress and Softube Fix Phaser manuals.
- Effects Explained (the sox_ng wiki), "Chorus" and "Phase shifting".
- research-modulation.md in jferments/mpfree-claude `fx/quality/` has the links.
