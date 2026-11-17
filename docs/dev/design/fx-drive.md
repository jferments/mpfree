<!--
covers: rust/mpfree-fx/src/drive.rs rust/mpfree-fx/src/character.rs rust/mpfree-fx/src/adaa.rs rust/mpfree-fx/src/oversample.rs
checked: 2026-11-17 c87022f
-->
# Drive and Character: design

**Status:** 2026-11-17, after FX quality round 2's Drive PR (#445). The two engines that add harmonics with a curve: **Drive** (`mpfree.fx.drive`, six curves from warm to wrecked) and **Character** (`mpfree.fx.character`, the colour of analog gear at low drive). The guitar amp and the fuzz pedal have pages of their own: [fx-amp.md](fx-amp.md) and [fx-fuzz.md](fx-fuzz.md). Every effect's short entry is in [fx.md](fx.md); the measured targets are in [fx-quality.md](fx-quality.md) (ids G1 to G11). The research notes and the plan this page draws on are in jferments/mpfree-claude `fx/quality/` (research-drive.md, plan-2.md section 4, baseline-2026-11-17.md).

## 1. What it is for

Drive turns a clean sound into one with more harmonics and rounder peaks. People reach for it for five jobs, from a whisper to a wreck:

| Job | Typical move | What matters most |
|---|---|---|
| Warmth and glue on a bus, drums, bass or a vocal | 0.5 to 3 % distortion (about -45 to -30 dB THD), often in parallel | Low-order harmonics only, no fizz, the level doesn't jump, the low end stays tight |
| Presence and loudness: a bass or kick that reads on small speakers, peaks rounded before a limiter | A few dB of soft clipping | Even and odd harmonics in the right mix; transients not dulled too much |
| A named colour: tape, tube, console, transformer | Low drive into a named curve with its tone shaping | The colour is recognisable: its harmonics and its tone curve |
| Overdrive and distortion on guitars, synth leads, 808s, breaks | 12 to 40 dB of gain into a clipper, a tone control after it | Dense but not harsh; no mud on chords; it cleans up as you play softer |
| Effect: fold, rectify (octave up), fuzz | Extreme curves | Wild but controllable: no runaway level, no aliasing that sounds broken rather than chosen |

Drive covers all five with six curves; Character covers the third with four small, cheap processors. The MPC's distortion effects open as Drive presets when a project or a picker name asks for them (`mpc.rs`): Distortion Overdrive and AIR Tube Drive as Tube glow, Distortion Fuzz and AIR Fuzz-Wah as Fuzz box (the wah as an Auto wah after it), Distortion Grimey as Foldback grit, AIR Distortion and Diode Clipper as Hard clip, Distortion Custom as Drive with a Mode to pick. Distortion Amp opens the Amp.

Jay's guide for the whole distortion family (17 Nov): rich, gnarly distortion must be possible, but the effects are not aimed at one style; defaults and presets stay broad.

## 2. Principles

**A curve's shape is its sound.** Drive and Character are memoryless waveshapers: each output sample is a fixed function of the input sample, `y = f(g x)`, with `g` the drive. What `f` does to a sine decides the harmonics:

- A curve that is **odd-symmetric** (`f(-x) = -f(x)`: tanh, a clamp) makes only odd harmonics (3rd, 5th, ...), heard as edgier and "hollow". An **asymmetric** curve (a tube's or a transistor's, tape's bias) adds even ones (2nd, 4th, ...), heard as fuller and "warmer" (Hamm 1973 is the classic comparison).
- A **smooth** curve's harmonics fall away fast with their order. A **hard corner** (a clamp, a fold's crest) has harmonics falling only as about 1/n, out to very high orders.
- **High orders are what sound harsh.** Listening studies found THD a poor predictor of how distortion sounds; weighting each harmonic by its order (Shorter's n²/4) or the curve's higher derivatives (Geddes and Lee's GedLee metric) predicts it much better. A little 7th is worse than a lot of 2nd.
- **Touch.** A soft curve's distortion rises about 2 dB for each dB of input, so playing softer cleans up; a hard clipper is clean, then suddenly not.

**Aliasing.** A curve makes harmonics above the session's Nyquist, and a digital curve folds them back as tones that aren't harmonics of the note: fizz on high notes and cymbals, tones that move the wrong way as a note bends. Lehtonen, Pekonen and Välimäki (JASA 2012) found aliases above the fundamental need up to 41 dB of attenuation to be inaudible on a 4 kHz sawtooth at only 60 dB SPL; louder playback needs more, and masking decides it. Two remedies, which MPFree uses together:

- **Oversampling.** Run the curve at 2 or 4 times the rate between half-band filters, so harmonics up to that rate's Nyquist are made and then filtered out instead of folding. Hard curves need far more than 4x on their own.
- **Antiderivative antialiasing (ADAA;** Parker, Zavalishin and Le Bivic, DAFx 2016; Bilbao, Esqueda, Parker and Välimäki 2017). Instead of `f` at each sample, play the average of `f` over the straight line from the last input to this one: `(F(x[n]) - F(x[n-1])) / (x[n] - x[n-1])`, with `F` the antiderivative of `f`. That is the curve heard through a one-sample box filter in continuous time, whose harmonics fall away far faster, so most of what would fold is gone before it folds. Its price: where two inputs are nearly equal the quotient loses precision (the fix is `f` at the midpoint); `F` grows like `|x|`, so differences of nearby values need 64-bit floats; and in a curve's straight part the average is the two-point mean `(1 + z^-1) / 2`, half a sample late and dull at the top (-2.4 dB at 10 kHz and -17 dB at 20 kHz at the base rate; at 2x the same droop sits above the audio band, where a short filter can undo it).

**Level.** Louder sounds better in any quick comparison, so a drive that gets louder as it turns always "wins", and the user can't judge the grit. Auto gain keeps the level where it was, so more drive means more grit. The catch is physics: a hard-driven curve's output level hardly depends on its input level, so no fixed gain holds every source at the dry's loudness. A fixed gain can be right at one level, and the useful one is a typical mix level on music-like material.

**Bias and DC.** An asymmetric curve turns a symmetric input into an output with a DC offset that follows the input's envelope: each drum hit becomes a pulse of energy under 15 Hz that eats headroom and pumps a limiter. Subtracting `f(bias)` takes off the static offset; a high-pass takes off the moving part.

**Mix.** Drive's wet sound is the same moment as the dry and, by design, in phase with it, so a linear Mix adds them without a dip (plan-2 section 2.1). Parallel distortion on drums is a common use (Mix 20 to 60 %).

**Mud.** Bass through a clipper modulates everything above it (intermodulation); on chords and full mixes that is mud. The usual cure is a high-pass or tilt before the curve (the Tube Screamer's 720 Hz, Decapitator's Low Cut) or splitting the band (Saturn). Drive doesn't have this yet (section 10).

## 3. Research

References studied (research-drive.md has the details and the sources): Soundtoys Decapitator (five analog styles, Drive, Punish, a tilt Tone before the curve, Low Cut before and High Cut after with a steep option, Auto gain, Mix), FabFilter Saturn 2 (28 styles, up to six bands, HQ modes at 8x and 32x), Ableton Saturator (Analog Clip, Soft Sine, Medium, Hard, Sinoid Fold, Digital Clip, a Waveshaper; Drive, a DC bias, a pre and post Color EQ) and Roar, the Tube Screamer (Electrosmash's circuit analysis: a 720 Hz high-pass in its clipping stage, 12 to 118 times the gain), and the MPC's own distortions. Tape's maximum output level is defined at 3 % third-harmonic distortion of a 1 kHz tone, a reference for how much distortion "tape at its limit" is.

What musicians expect, and where Drive stands:

| Feature | Common practice | Drive and Character |
|---|---|---|
| Curves | Soft, hard, tube, tape, fuzz, fold, rectify | Drive: Soft, Hard, Tube, Foldback, Fuzz, Rectify. Character: Console, Tape, Iron, Soften |
| Drive range | 0 to about 40 dB; fuzz more | Drive 0 to 36 dB; Character 0 to 12 dB |
| Tone after the curve | Low-pass or tilt; a steep cab-like cut | A 12 dB per octave low-pass, 500 Hz to 20 kHz |
| Before the curve (low cut, tilt) | Decapitator, Tube Screamer, Saturator's Color | None yet |
| Bias (even harmonics) | Saturator's DC, tube styles | Bias in every curve but Soft (decision 10.1) |
| Auto gain | Most | Yes, on music-like noise |
| Mix | All | Yes, phase matched |
| Oversampling choice | HQ modes | Eco 2x, HQ 4x, both with ADAA |
| Multiband, dynamics, feedback | Saturn, Roar | No |

The MPC's own distortions have not been measured: FXProbe, the tool that records the factory MPC's AIR effects ([fxlab.md](fxlab.md)), didn't run in the 16 Nov device round, so there are no AIR captures to compare against yet.

**The baseline** (main `5be7b53`, 44.1 kHz, before round 2's Drive PR) found a clean small-signal response, honest harmonic profiles and no DC, but: aliasing in Eco (a 5 kHz full-scale sine: -53 dB in Soft, -38 dB in Hard) and in Character, which had no oversampling (-34 dB in Tape); auto gain matched on a -12 dBFS sine, so mix-level pink got louder with Drive (+3.4 LU at 36 dB, +6 LU in Foldback and Rectify); Bias did nothing in four of six modes; a mode change clicked (31 dB over the sound) and a Quality change too (68 dB); drum hits through Tube and Fuzz at Bias 100 put 13 to 14 dB of their energy under 15 Hz.

## 4. Our engines

### Drive

The signal path, per side:

```
in ─ × drive ─ up 2x (Eco) or 4x (HQ) ─ + bias ─ ADAA curve ─ − f(bias) ─ flat FIR ─ down ─ × auto gain ─ Tone ─┐
 └──────────── dry, through the same phase and delay as the wet ──────────────────────────────────────────────── Mix ─ 17 Hz high-pass ─ × Output ─ out
```

**The curves** (`base` and `base_big` in `drive.rs`). Each is `f(v)` with its antiderivative `F(v)`, zero at 0:

| Mode | `f(v)` | `F(v)` | What it sounds like |
|---|---|---|---|
| Soft | `tanh v` | `log cosh v` | Odd harmonics that fall fast: warm saturation to smooth overdrive. Bias is ignored: Soft with a bias is Tube |
| Hard | `v` clamped to ±1 | `v²/2` inside, `|v| - 1/2` outside | A diode-like hard clip: odd harmonics to high orders, edgy |
| Tube | `tanh v` with Bias (default 20 %) | as Soft | One half clips first: even and odd harmonics, fuller |
| Foldback | `sin(πv/2)` | `(2/π)(1 - cos(πv/2))`, written `sin²(πv/4)/(π/4)` to avoid cancellation | The wave folds back on itself past full scale: dense, metallic, bell-like at high drive |
| Fuzz | `sign(v)(1 - e^(-2|v|))` | `|v| + (e^(-2|v|) - 1)/2` | A sharp-kneed exponential: buzzy, sustaining, even harmonics with Bias |
| Rectify | `tanh |v|` | `sign(v) log cosh |v|` | Full-wave rectified: an octave up, the fundamental gone |

**Bias** (decision 10.1, Jay 17 Nov). Every curve but Soft plays `f(g x + b) - f(b)`, with `b` = Bias / 100 (-1 to 1) added after the drive gain, so it moves the working point along the curve's own scale. Subtracting `f(b)` keeps silence silent and takes off the static offset. Hard then clips one side sooner (a mismatched diode pair), Foldback folds off centre, Rectify folds at the bias, as Tube and Fuzz already did. In Soft the knob is dimmed with "Soft doesn't use it". The alternative, dimming it wherever it did nothing, kept every sound but left four modes with no asymmetry at all.

**Antialiasing: ADAA inside the oversampling** (`adaa.rs`, FX quality round 2, targets G1 and G2). Each oversampled sample of the curve is `(F(v) - F(v1)) / (v - v1)`, `F` worked out in 64-bit floats (it grows like `|v|`, and the difference of two nearby values needs the extra digits), with `f` at the midpoint when the step is under a threshold set so the rounding stays under 1e-8 and the midpoint's error under 1e-5 of the curve's range. The two-point mean that ADAA is in a curve's straight part is undone by a short symmetric FIR after it (`Flat2x`: 8 taps at 2x, flat within 0.01 dB to 20 kHz; `Flat4x`: 4 taps at 4x, within 0.03 dB), which also makes the pair a whole number of base frames late, so the fixed latency and the phase-matched dry still hold.

- **Eco** (2x): polyphase IIR half-bands, five first-order all-pass sections each way (transition band 0.06; flat to 0.19 of the 2x rate, at least 71 dB down from 0.31), a fifth of the FIR pair's cost on the MPC's ARM (Jay's call, 13 Nov). An IIR pair isn't linear phase, but the pair with a straight line between is exactly one all-pass, so the dry side runs through that all-pass (`IirPhase`) and any Mix adds wet and dry in phase at every frequency. Its aliasing comes from the curve, not the filters: six or seven sections changed nothing but Foldback.
- **HQ** (4x): linear-phase FIR half-bands (95 taps for the first doubling, 31 for the second), so its wet keeps the dry's phase by a plain delay.
- **Latency** is 55 frames in both, Eco padded to HQ's, so switching quality never moves the sound in time.

Why both at once: ADAA alone at the base rate leaves hard curves near -40 dB and dulls the top; oversampling alone needs 8x or more for a hard edge. First-order ADAA at 2x took Soft's 5 kHz alias from -53 to -69 dB; at 4x Hard from -50 to -69 dB.

Turned down: second-order ADAA (a second antiderivative per curve, worse conditioning, more cost, and first order already met the smooth curves' targets); 8x oversampling (by estimate about twice HQ's filtering cost); polyBLAMP corrections at the clipper's corners (only for hard corners, where ADAA covers every curve the same way); more half-band sections (measured: no change).

**Auto gain on music-like noise** (`auto_gain`, G4). The gain that brings Gaussian noise at -18 dBFS RMS back to its own RMS after the curve, its DC removed as the output's high-pass does. Pink noise and a dense mix have roughly that Gaussian spread of sample values, and -18 dBFS RMS is a usual mix-level reference (fxlab's `lu_pink18`); the old reference, a -12 dBFS sine, peaks far less often than music, so more drive made music louder. The average is a 128-point quadrature over ±8 standard deviations, worked out only when Drive, Bias or Mode change (never per sample), capped at 36 dB. Foldback folds too often at high drive for the points to follow, so it uses the closed form: for `x` normal with spread `s`, `E sin(a(x + b)) = sin(ab) e^(-a²s²/2)` and `E sin² = (1 - cos(2ab) e^(-2a²s²)) / 2`. The gain glides over 20 ms.

**Tone**: a state-variable low-pass (12 dB per octave, Q 0.707) at the base rate after the down-sampler, 500 Hz to 20 kHz; it glides in log frequency, its coefficients worked out each frame while it moves.

**A change of Mode** runs both curves for 10 ms and crossfades. The incoming curve's ADAA state is seeded from the last input (with its own bias), so its first average doesn't see a jump. **A change of Quality** starts the other path from silence, runs it 128 frames unheard so its filters are full of the music, then crossfades over 10 ms; the old path stops once it is silent.

**The low end.** A fourth-order Butterworth high-pass at 17 Hz on the output, dry and wet together (so Mix adds them in phase at the bottom too), replaced the old 10 Hz first-order DC blocker, which passed an asymmetric curve's envelope-following offset. Within 0.1 dB from 25 Hz up; Mix 0 is the input with nothing under 17 Hz; it rings out 100 dB in 0.3 s, which is the tail Drive reports.

**Numerics on the MPC's ARM.** `tanh` is a rational fit within 2e-7 (Eigen's single-precision one) instead of the C library's; the antiderivatives use MPFree's own 64-bit `exp` and `ln` (`adaa::exp`, `adaa::ln`, within 2e-15 of the library's; the library's were a quarter of Drive's cost); Eco's half-bands run unrolled (`oversample::IirPair`, the same output to the bit) and flush tiny states every 32 frames counted from the stream's start, so any block size sounds the same.

**Level change** is a meter: the output's mean square over the input's, each followed over 0.3 s, in dB.

### Character

Four small processors in the spirit of Airwindows' single-purpose plug-ins (original code, not a port), sharing one Amount knob (0 to 12 dB of drive into the curve), Output and Mix. Each mode drives its curve by the Amount's gain and scales the result back by the same gain, so quiet signals come out at the level they went in, in every mode: Amount changes the colour and how hard peaks are held, not the level of quiet parts.

| Mode | What it does | Why this way |
|---|---|---|
| Console | An odd quintic shaped like `sin` up to π/2 and flat beyond: unity slope at 0, a flat top, so it never folds back | A channel's rounded warmth: odd harmonics only, a gradual top, the cheapest curve with a flat top |
| Tape | A rational soft clip, `x/9 + (8/3)x/(x² + 3)` up to ±3 and ±1 beyond; a head bump at 75 Hz (Q 1.2, up to +3 dB at Amount 100); a top-end loss whose corner falls from 22 kHz to 11 kHz with Amount | Tape's softer knee, its low bump and its loss of top as it is pushed |
| Iron | Everything under 120 Hz is driven twice as hard as the rest, with an even harmonic (0.15 of the low band squared at Amount 100), whose DC alone a 10 Hz blocker takes off | A transformer saturates its lows first and thickens them |
| Soften | A smooth slew limit: a full-scale sine at 2.5 kHz (Amount 100) to 40 kHz (Amount 0) has its steepest steps held to about four fifths | Takes the edge off loud, harsh highs while quiet highs and loud lows pass untouched |

Since round 2 every mode runs at twice the rate between Drive's Eco half-bands, with ADAA on the Console, Tape and Iron curves (G1: Console and Tape aliased at -41 and -34 dB at 5 kHz at the base rate, now -99 and -95). The dry side takes the half-bands' all-pass and the same delay, so any Mix adds the two in phase; the latency is 4 frames (it was 0). Tape's top-end loss is placed at twice the rate to lose what it lost at the base rate at 16 kHz, so saved Tape sounds keep their colour. A mode change crossfades over 10 ms; the coefficients that follow Amount are worked out every 16 frames while it glides, on a grid counted from the stream's start.

## 5. Controls

| Drive control | Range (default) | Law and smoothing |
|---|---|---|
| Mode | Soft, Hard, Tube, Foldback, Fuzz, Rectify (Soft) | 10 ms crossfade |
| Drive | 0 to 36 dB (12) | Linear in dB, 20 ms glide; auto gain follows it |
| Tone | 500 Hz to 20 kHz (20 kHz, open) | Log, glides in log frequency |
| Bias | -100 to 100 % (20) | 20 ms glide; dimmed in Soft |
| Mix | 0 to 100 % (100) | Linear, 20 ms glide |
| Output | -24 to +12 dB (0) | 20 ms glide |
| Auto-gain | Off, On (On) | The gain glides over 20 ms |
| Quality | Eco 2x, HQ 4x (Eco) | Warm-up then a 10 ms crossfade |
| Level change | -48 to +48 dB, read only | 0.3 s meter |

| Character control | Range (default) | Law and smoothing |
|---|---|---|
| Mode | Console, Tape, Iron, Soften (Console) | 10 ms crossfade |
| Amount | 0 to 100 % (40), up to 12 dB of drive | Glides; coefficients every 16 frames while it moves |
| Output | -12 to +12 dB (0) | Glides |
| Mix | 0 to 100 % (100) | Linear, glides |

Parameter ids never move: new ones are appended, so saved projects load.

## 6. Defaults and presets

**Drive's default** is Soft at 12 dB, Tone open, Eco, Mix 100, auto gain on: an insert that is audibly warmer and no louder, cheap enough to leave on. Eco is the default because it is a fifth of HQ's half-band cost and, with ADAA, meets the smooth curves' alias target (Soft's 5 kHz alias -69 dB); HQ is for hard curves on bright material. Bias 20 % gives Tube its even harmonics out of the box and does nothing in Soft.

**Drive's presets** each cover one job, spaced from subtle to extreme, and each plays within its loudness target on pink at -18 dBFS:

| Preset | Settings | Job |
|---|---|---|
| Warm tape | Soft, 6 dB, Tone 12 kHz | Gentle saturation and a softened top (its Bias 10 no longer does anything in Soft) |
| Tube glow | Tube, 12 dB, Tone 9 kHz, Bias 40 | Even-harmonic overdrive (the MPC's Overdrive and AIR Tube Drive) |
| Hard clip | Hard, 18 dB, Tone 16 kHz, Bias 0 | A symmetric diode clip (AIR Distortion, Diode Clipper) |
| Fuzz box | Fuzz, 30 dB, Tone 4.5 kHz, Bias 30 | Buzzy, sustaining fuzz with the fizz rolled off |
| Foldback grit | Foldback, 20 dB, Tone 8 kHz | Metallic fold (the MPC's Grimey) |
| Octave up | Rectify, 6 dB, Tone 7 kHz | An octave up on single notes |

**Character's default** is Console at Amount 40 (on a -12 dBFS sine, a third harmonic about 42 dB down): a colour you notice when you bypass it, not before. Its presets are two depths of each colour that has depths (Console warmth 30 and Hot console 85, Tape glue 45 and Pushed tape 85) plus Iron bass (60) and Soft highs (Soften, 50).

## 7. Measuring quality

`fxlab quality --class drive` measures every mode at its defaults, in both qualities, and every preset ([fx-quality.md](fx-quality.md), Drive (G)). The targets and why:

| Id | What | Target |
|---|---|---|
| G1 | Worst alias of a 5 kHz sine at 0 dBFS, smooth curves | ≤ -65 dB in Eco and Character, ≤ -90 dB in HQ |
| G2 | The same, hard edges (Hard, Fuzz, Foldback; Rectify in dBFS) | ≤ -50 dB Eco, ≤ -60 dB HQ |
| G4 | Loudness on pink at -18 dBFS across Drive 0 to 36 and Amount 0 to 100 | Within ±1 LU of the dry (Foldback and Rectify ±3) |
| G5 | Loudness on loud pink and on drum hits | Reported (rounding loud peaks loses loudness by design) |
| G6 | Bias 0 against 100 | The output changes by at least -20 dB of itself, every mode but Soft |
| G7 | A Mode or Quality change mid-note | ≤ 3 dB over the sound |
| G8 | Small-signal and Mix 50 % response | Flat within ±0.5 dB, 20 Hz to 20 kHz |
| G9 | Energy under 15 Hz on hits at Bias 100; DC | ≤ -20 dB of the hit; DC ≤ -100 dBFS |
| G10 | H2 against H3, the high-order share, THD by level, SMPTE and CCIF IMD | Reported |
| G11 | fxlab's zipper on every control | ≤ 3 dB |

After the Drive PR, 106 of 109 are met (76 before). The three left: Fuzz's and Rectify's Eco alias at 5 kHz (-49.99 dB and -49.3 dBFS against -50: at 2x a hard edge's harmonics fold before first-order ADAA can take them out; HQ meets both), and Tape at Amount 100 plays pink 1.77 LU under the dry, its own head bump and top-end loss, left as designed. The ratchet (`rust/mpfree-fxlab/quality/ratchet.tsv`) holds every number, so none can get worse without a blessed, explained change.

Listening: the Drive PR's blind, loudness-matched A/B page (main against the PR, five cases: a soft lead, Hard clip, Fuzz box, Pushed tape, and mode changes while playing) is https://claude.ai/artifact/NgARB8Na4kZ6ZKzwt5qq4L.

## 8. Cost

The instruction gate (`rust/mpfree-bench/instructions.txt`; rustc 1.97.0, a 128-frame stereo block at 44.1 kHz) counts Drive (Eco 2x) at 202.6k instructions (154.6k before ADAA, 1.31 times), Drive (HQ 4x) at 430.3k (272.3k, 1.58 times), and Character at 97k (Soften) to 150k (Tape), 4.2 to 5.2 times its old base-rate cost. A 128-frame period at 44.1 kHz is about 4.7 million cycles of one of the MPC's cores, so these are a few percent each; the device round measures the real share (drive-device-checks.md in project files). Levers if it is too dear: Iron and Soften aliased little at the base rate (-140 and -72 dB at 5 kHz), so those two could go back to it; three half-band sections each way in Character would save 15k but let Tape alias at -61 dB.

## 9. Pitfalls and tests

| Pitfall | Guard |
|---|---|
| Fizz from aliasing | ADAA in 2x or 4x; G1 to G3 on the ratchet |
| Level that follows Drive | Auto gain on Gaussian noise; G4 |
| A knob that does nothing | Bias in every curve but Soft, dimmed there; G6; `bias_acts_in_every_mode_but_soft` |
| Clicks on a mode or quality change | Crossfades with seeded ADAA; G7; `a_change_of_mode_or_quality_does_not_click`, Character's `a_change_of_mode_crossfades_without_a_click` |
| Thumps from DC | `f(x + b) - f(b)` and the 17 Hz fourth-order high-pass; G9 |
| Dull top or comb filtering in parallel | Flat FIRs after ADAA; the dry through the same all-pass and delay; G8; `quiet_signals_pass_flat_in_both_qualities`, `drive_eco_blends_with_the_dry_side_at_any_mix` |
| ADAA blowing up when the input holds still | The midpoint fallback; `a_held_input_holds_still`, `adaa::a_held_input_gives_the_curve_and_a_ramp_its_average`, `huge_and_odd_inputs_stay_finite` |
| An antiderivative that doesn't match its curve | `every_antiderivative_matches_its_curve` (each `F`'s numerical slope against `f`, from -10 to 10) |
| Auto gain wrong | `auto_gain_holds_gaussian_noise_at_its_level` |
| Character colours drifting | `quiet_signals_keep_their_level_in_every_mode`, `console_holds_loud_peaks_with_odd_harmonics_and_iron_adds_even_ones`, `tape_lifts_its_head_bump_and_loses_a_little_top`, `soften_takes_the_edge_off_loud_highs_only` |
| A block size changing the sound | `same_output_for_any_block_size` (`tests/fx.rs`, `tests/coverage.rs`) runs every effect under random automation in blocks from 1 frame to the maximum and compares the outputs bit for bit |

## 10. History and next

- **Before round 2:** Drive's six curves inside 2x or 4x oversampling with no ADAA, auto gain on a -12 dBFS sine; Character at the base rate; Eco moved from FIR to IIR half-bands on 13 Nov for cost.
- **Round 2, Drive PR (#445, 17 Nov):** ADAA in both qualities and in Character at 2x; auto gain on Gaussian noise; crossfades on Mode and Quality; Bias in every curve but Soft (decision 10.1); the 17 Hz fourth-order high-pass.
- **Next, from plan-2 section 4.2, each appended with a default that keeps today's sound:** **Tight**, a high-pass before the curve (20 Hz, off, to 1 kHz), so chords and full mixes don't go to mud; a 24 dB per octave choice for Tone, to take the fizz off the hard modes; **Emphasis**, a tilt before the curve undone after it, to choose whether highs or lows distort first. Device-round results may move Iron and Soften back to the base rate. Fuzz's and Rectify's Eco alias would need 4x or a second-order ADAA to meet G2.

## 11. Sources

- T. Hamm, "Tubes versus transistors: is there an audible difference?", JAES 21(4), 1973.
- E. Geddes and L. Lee, "Auditory perception of nonlinear distortion", AES 115th Convention, 2003; D. E. L. Shorter, "The influence of high-order products in non-linear distortion", Electronic Engineering 22, 1950.
- H.-M. Lehtonen, J. Pekonen and V. Välimäki, "Audibility of aliasing distortion in sawtooth signals and its implications for oscillator algorithm design", JASA 132(4), 2012.
- J. Parker, V. Zavalishin and E. Le Bivic, "Reducing the aliasing of nonlinear waveshaping using continuous-time convolution", DAFx 2016; S. Bilbao, F. Esqueda, J. Parker and V. Välimäki, "Antiderivative antialiasing for memoryless nonlinearities", IEEE Signal Processing Letters 24(7), 2017; J. Chowdhury, "Practical considerations for antiderivative anti-aliasing"; M. Vicanek, "Antialiased distortion".
- Soundtoys, Decapitator manual; FabFilter, Saturn 2 release notes; Ableton, Live 12 audio effect reference (Saturator, Overdrive, Pedal, Roar); Electrosmash, "Tube Screamer circuit analysis".
- ITWissen, "Maximum output level (MOL)" of magnetic tape (3 % third harmonic at 1 kHz).
