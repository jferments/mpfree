<!--
covers: rust/mpfree-fx/src/comp.rs rust/mpfree-fxlab/src/dynamics.rs
checked: 2026-11-17 60bf82e
-->
# Compressor: design

**Status:** 2026-11-17, FX quality round 2's Compressor PR. **Compressor** (`mpfree.fx.compressor`) is a feed-forward, soft-knee compressor with four styles (Clean, Opto, Bus, FET), auto makeup, Range, Link, a side-chain key and filter, and a parallel Mix. The Channel strip effect's compressor is this engine at its Clean style. Its short entry is in [fx.md](fx.md); its measured targets are in [fx-quality.md](fx-quality.md) (ids K1 to K15). The research notes and the plan this page draws on are in jferments/mpfree-claude `fx/quality/` (research-compressor.md, plan-2.md section 6, decision 10.3). The Limiter, Gate, Ducker and Transient shaper are separate engines.

## 1. What it is for

A compressor turns loud parts down by a ratio above a threshold, then lets the whole thing back up with makeup, so the level varies less. Jobs and the settings people use for them:

| Job | Typical settings | What matters most |
|---|---|---|
| Levelling a vocal or bass | 3:1 to 4:1, attack 5 to 20 ms, release 50 to 150 ms, 2 to 6 dB of reduction; RMS or opto | Smooth, unnoticed gain riding; no pumping; consonants and plucks kept |
| Punch on drums | 4:1 to 10:1, attack 10 to 30 ms (the hit passes), release timed to the groove, 3 to 8 dB | The attack shape: the first milliseconds pass, then the body is held |
| Glue on a bus or mix | 1.5:1 to 2:1 (SSL-style 2 or 4), attack 10 or 30 ms, release auto or 0.1 to 0.6 s, 1 to 3 dB; side-chain high-pass 60 to 150 Hz | Transparent, no distortion, a steady image, the kick doesn't pump the mix |
| Parallel ("New York") | 8:1 to 20:1, fast, 10 to 20 dB, mixed in at 20 to 50 % | A dense wet signal that adds in phase |
| Pumping and ducking | High ratio, fast attack, release to tempo, keyed from a kick | Clean, rhythmic, predictable |
| Colour and smash | Extreme (an 1176 with all buttons in) | Harmonic colour and the time constants' feel |

The MPC's compressors open as presets (`mpc.rs`), each now on a style of its own: Compressor Master as Gentle glue (Bus), Compressor VCA as Drum punch (Bus), Compressor Opto as Vocal level (Opto), Oldskool as Smash (FET); Compressor and AIR Compressor open the defaults.

## 2. Principles

**The gain computer** (Giannoulis, Massberg and Reiss, JAES 2012, the reference this design follows). Work in decibels: for an input level `x` dB, threshold `T`, ratio `R` and knee width `W`, the reduction is 0 below `T - W/2`, `(1 - 1/R)(x - T)` above `T + W/2`, and in between the quadratic `(1 - 1/R)(x - T + W/2)² / (2W)` that joins them smoothly. The output is the input minus the reduction plus makeup. Computing the gain in dB and smoothing it there makes attack and release behave the same at any level.

**The detector decides how the gain moves**, and most of a compressor's character is there. Giannoulis and colleagues compare the options: a **branching** detector (attack while the level is above the envelope, release otherwise) gives direct control of the times but on a periodic signal only attacks near each peak, so it settles short of the drawn curve and its gain ripples inside each cycle; a **decoupled** detector puts an instant-attack peak hold with the release first, then the attack smoothing; the **smooth** variants release toward the input rather than toward zero. Their recommendation, which this engine follows: feed-forward, the detector in the log domain after the gain computer, and the smooth decoupled peak detector, for artefact-free compression.

**Times need a stated convention.** Units disagree on what "attack 10 ms" means: a time constant (63 %), a 10 to 90 % rise (τ ln 9, 2.2 times longer), or time to a set amount of reduction. MPFree uses the time constant for both Attack and Release, says so in the manual, and measures it.

**Gain riding inside a cycle is distortion.** A fast detector moves the gain within each cycle of a low note: gain-modulation distortion, worst on bass. THAT Corporation's analysis puts an RMS detector with τ = 35 ms at infinite ratio at about -40 dB THD at 60 Hz and -65 dB at 1 kHz, falling about 6 dB per octave; Universal Audio cite about 0.7 % (-43 dB) as a common audibility line for music, and note that distortion confined to the first 5 ms (an attack) is much less audible.

**Program-dependent release.** Classic units come back quickly after a short hit and slowly after long, heavy reduction: the LA-2A's light cell (about half back in 60 ms, the rest over 0.5 to 5 s, longer after heavy compression), the SSL bus compressor's Auto release. A fixed release is either too slow for hits (the mix ducks after every snare) or too fast for sustained material (it pumps and breathes).

**Level and fair comparison.** Compression without makeup is quieter, and louder sounds better, so a compressor A/B'd against bypass at a lower level loses unfairly. Auto makeup restores the level the curve takes from a sound at a typical level, so the user hears the change in dynamics, not in loudness.

**Linked stereo** keeps the image still: both sides take the louder side's reduction. Unlinked lets each side ride itself, which widens or wanders.

**Transparency when idle.** At Ratio 1, below the knee, or at Mix 0, the output should be the input bit for bit, so inserting a compressor costs nothing until it works. Its Mix is a gain on the same samples, so a parallel blend adds in phase.

## 3. Research

References studied (research-compressor.md has the details and the sources): Giannoulis, Massberg and Reiss's tutorial and analysis; THAT Corporation on RMS time constants and ripple; Universal Audio's analysis of dynamics devices and the 1176 manual; the classic units (1176 FET: attack 20 to 800 µs, release 50 ms to 1.1 s, ratios 4, 8, 12, 20; LA-2A opto: about 10 ms attack, the two-stage release; SSL G bus: attack 0.1 to 30 ms in steps, release 0.1 to 1.2 s and Auto, ratios 2, 4, 10; vari-mu: a knee whose ratio rises with level); FabFilter Pro-C 2 (styles Clean, Classic, Opto, Vocal, Mastering, Bus, Punch, Pumping; knee, range, look-ahead, hold, auto release, auto gain, side-chain EQ, mid/side); Ableton Compressor and Glue Compressor; and the MPC's compressors. The MPC's AIR Compressor hasn't been measured (FXProbe hasn't run).

What musicians expect, and where Compressor stands now:

| Feature | Common practice | Compressor |
|---|---|---|
| Threshold, ratio, attack, release, knee, makeup | All | Yes; ratio 1 to 20, knee 0 to 24 dB, attack 0.1 to 100 ms, release 10 ms to 2 s |
| Auto makeup | Most | Auto makeup (off by default, decision 10.3) |
| Program-dependent release | SSL Auto, LA-2A, Pro-C | In the Opto, Bus and FET styles |
| Styles | Pro-C's styles; one plug-in per classic unit | Style: Clean, Opto, Bus, FET |
| Peak and RMS detection | Common | Detect: Peak, RMS |
| Side-chain filter and external key | Common | SC filter (a high-pass, 20 to 500 Hz) and Sidechain |
| Range | Pro-C, gates | Range |
| Stereo link amount, mid/side | Pro-C | Link; no mid/side yet |
| Look-ahead, hold | Pro-C | No: the Limiter looks ahead (section 4) |
| Mix | Common | Mix |

**The baseline** (main before this PR, with fxlab's timing measure fixed as described in section 7): a 1 kHz sine at 0 dBFS got 0.95 dB less reduction than drawn (the branching detector on the raw signal attacks only near each peak); attacks took about 1.5 times their label (1.58, 1.48, 1.49 and 1.51 at 1, 10, 30 and 100 ms); 10 dB of reduction on a 50 Hz sine distorted at -43.8 dB and on 100 Hz at -49.8 dB at the default times; Ratio 1 wasn't quite bit for bit (-132 dBFS); there was no auto makeup, so the defaults played pink at -18 dBFS 3.0 LU under the dry and the presets from +1.9 to -8.2 LU; one character only. Release was already honest (within 4 % from 50 ms up), as were the meter and the link.

## 4. Our engine

Per frame, linked (Link 100, the default):

```
key (the input or the side chain) ─ [SC filter: 12 dB/oct high-pass] ─ level (Peak, or RMS over 10 ms) ─ gain computer (curve, capped by Range) ─ hold ─ attack ─ [slow stage] ─┐
in ─────────────────────────────────────────────────────────────────────────────────────── × gain(makeup − reduction), blended by Mix ─ [+ FET colour] ─ out ◄──────┘
```

**The level.** Peak reads `|x|` of the louder side (or each side, unlinked) in dB through a fast log2 fit (within 0.0001 dB; `log10` was most of the compressor's cost on the MPC's ARM). RMS is a one-pole mean square over 10 ms, calibrated 3 dB up so a steady sine reads the same in both: switching changes how hard short hits push it, not where the threshold sits (a 1 ms click every 50 ms: Peak reduces at least 4 dB more).

**The SC filter** is a 12 dB per octave high-pass from 20 to 500 Hz on what the detector hears (off, and not run, at 20 Hz), so a kick's low end doesn't pump a mix bus.

**The detector** (`detect`): the smooth decoupled peak detector, on the reduction the curve asks for, `want`:

- the **hold** jumps up to `want` at once and lets go toward it at the release: `hold = max(want, r·hold + (1 - r)·want)`;
- the **attack** smooths the hold with a one-pole of time constant Attack: `env = a·env + (1 - a)·hold` (two such poles in Opto).

Because the hold sits on the reduction, not the raw signal, a steady tone holds the whole reduction drawn, and the gain barely moves within a bass cycle. The release runs through the attack after it, so on its own it would be slow by about the attack's time; `release_tau` solves, by bisection over the closed-form step response of the cascade (the release pole, then one or two attack poles), for the hold's own time constant so the two together reach 63 % of the way back in the labelled Release. Where the attack alone already takes longer than the release, the hold lets go at once and the attack sets the pace.

**The gain** is `makeup - reduction` in dB, turned into a factor by a fast exp2 fit, and applied as `x · (1 + mix · (g - 1))`: a linear blend of the same samples, so Mix adds wet and dry in phase. Below the knee, at Ratio 1 and at Mix 0 the reduction is exactly zero and the factor exactly 1, so the output is the input bit for bit.

**Styles** (`comp::STYLES`; decision 10.3, Jay 17 Nov: four styles). Clean is the curve and times as set. The other three add a **slow stage**, a program-dependent release in the spirit of the units they are named after: it follows the detector's reduction, charging over a stretch of reduction (its charge time) and letting go at its own slow release (a multiple of Release, within limits); the reduction applied is the larger of the fast detector's and a share of the slow stage's. After a short hit the slow stage hasn't charged and the release is Release; after long, heavy reduction a share of it comes back slowly.

| Style | Slow stage: share, charge, release | Also | After it |
|---|---|---|---|
| Clean | None | | Exactly the knobs |
| Opto | Half; 1 s; 10 × Release within 0.5 to 5 s | Two attack poles (an eased onset; still 63 % at Attack, each pole Attack / 2.146); a knee of at least 12 dB (the cell's ratio grows with level) | An LA-2A's light cell: smooth levelling, slow to let go after long compression |
| Bus | Half; 0.5 s; 3 × Release within 0.3 to 2 s | | An SSL bus compressor's Auto release: quick after a hit, slower under a steady mix, glue without pumping |
| FET | A third (0.35); 0.3 s; 4 × Release within 0.2 to 2 s | Colour: second and third harmonics as it turns down | An 1176's grit, more the harder it works |

**FET colour.** The compressed sample `y` is read against its own recent peak `p` (a peak follower with a 50 ms fall), `u = y / p`, and passed through the Chebyshev polynomials that turn a full-scale tone into its pure second and third harmonics: `T2(u) = 2u² - 1` (its DC taken out by a 10 Hz high-pass) and `T3(u) = 4u³ - 3u`, each scaled back by `p`. Because the result is homogeneous in level, the harmonics are a fixed share of the sound at any level, and makeup inside it is harmless. Each is weighted 0.01 times the colour amount, which is Mix times `min(reduction / 20 dB, 1)`: -40 dB each at 20 dB of reduction, about -43 dB THD at 10 dB, nothing while it isn't compressing. The amount glides over 20 ms, so switching to or from FET doesn't click. Turned down: a plain waveshaper on the signal (its distortion would follow the input's level, not how hard the compressor works; the Chebyshev form follows the reduction, and at the base rate puts a 5 kHz tone's alias at -84.6 dB); a first, weaker colour that reached only -56 dB THD at 10 dB of reduction, too little to hear as FET grit.

**A style change** carries the reduction on from where it was (leaving Opto, the second attack pole's state becomes the first's), clears the slow stage, and glides FET's colour in or out, so nothing clicks.

**Auto makeup** (decision 10.3: a toggle that starts off, so saved mixes keep their level and the presets keep their makeup). When on, it adds the reduction the curve asks for at a mix-level sound on top of Makeup: pink noise at -18 dBFS RMS, which a Peak detector reads near -10 dBFS and an RMS one near -14.5 dBFS, so the reference follows Detect. It is static (from the curve, not the signal), so it never pumps; the drawn curve includes it.

**Range** caps the reduction (1 to 60 dB; 60 is no cap): the gate-like "most it will take" of Pro-C and most gates, for gentle levelling that never crushes. The curve and the meter follow it.

**Link** (0 to 100 %, default 100). At 100 the louder side's level drives one detector for both; below it each side has its own detector and each side's reduction is pulled toward the louder side's by Link. At 0 each side rides itself.

**The meter** (Reduction) is the reduction applied, read once a block.

**One loop per detector setting.** The SC filter on or off, Peak or RMS, and linked or not are compile-time choices (eight monomorphs of `run_with`), so the default costs no branches for the options it doesn't use. FET's colour runs only while it has an amount, checked each frame once it is on, so where the glide reaches zero doesn't depend on the block size.

**Turned down: look-ahead.** A chain slot's dry delay is sized from the node's latency when the effect is inserted, so a latency that a knob turns on and off would need the chain to change first. Without it, the first milliseconds of a hit pass before the gain comes down (11 to 13 dB of overshoot on a -40 to 0 dBFS step at attacks of 1 and 10 ms): the punch drum users set a compressor for. The Limiter looks ahead for when that isn't wanted. Also turned down for now: auto release as a separate toggle (it would duplicate Bus); mid/side; hold.

## 5. Controls

| Control | Range (default) | Law and smoothing |
|---|---|---|
| Threshold | -60 to 0 dB (-18) | 20 ms glide |
| Ratio | 1 to 20 (4) | Log |
| Attack | 0.1 to 100 ms (10) | Log; a time constant (63 %) |
| Release | 10 ms to 2 s (150) | Log; 63 % back, through the attack |
| Knee | 0 to 24 dB (6) | At least 12 in Opto (the drawing shows it) |
| Makeup | 0 to 24 dB (0) | 20 ms glide |
| Mix | 0 to 100 % (100) | Linear, 20 ms glide |
| Sidechain | Off, On (Off) | The key from another strip |
| SC filter | 20 Hz (off) to 500 Hz | 12 dB per octave high-pass on the detector |
| Detect | Peak, RMS (Peak) | |
| Auto makeup | Off, On (Off) | Glides with Makeup |
| Style | Clean, Opto, Bus, FET (Clean) | The reduction carries on; FET's colour glides |
| Range | 1 to 60 dB (60, no cap) | |
| Link | 0 to 100 % (100) | |
| Reduction | -60 to 0 dB, read only | The meter |

Ids 0 to 10 are the first version's; Auto makeup, Style, Range and Link (11 to 14) were appended, and their defaults (off, Clean, no cap, linked) keep a saved compressor's behaviour; its sound changes only by the new detector (section 10). The page: Threshold and Ratio as heroes with Attack, Release, Knee and Range, and Style as a row, in the Compressor section with the curve and timing drawings and the meter; Detect, Sidechain, SC filter and Link in Detector; Makeup, Auto makeup and Mix in Output. The curve card shows the knee in use, the style, the range and auto makeup's gain; the timing card says "slower after long reduction" for the styles that are.

## 6. Defaults and presets

**The defaults** (-18 dB, 4:1, 10 ms, 150 ms, knee 6, Clean, Peak, linked, no makeup) are the middle of the levelling settings in section 1: a compressor that does something audible on a mix-level source without colour. With no makeup it plays pink at -18 dBFS 6.5 LU under the dry (the new detector reaches the full drawn reduction; the old one fell short), which is why Auto makeup is one press away; with it on, -0.55 LU.

**Presets** each set a style the job is known for and are as loud as the dry on pink at -18 dBFS with their own makeup (and within 0.8 LU with auto makeup instead):

| Preset | Settings | Style | Job |
|---|---|---|---|
| Gentle glue | -14 dB, 2:1, 30 ms, 300 ms, knee 10, +0.5 dB, SC filter 90 Hz, RMS | Bus | Mix bus glue that follows the mix's body, not the kick (the MPC's Compressor Master) |
| Drum punch | -20 dB, 4:1, 20 ms, 100 ms, knee 4, +7.5 dB | Bus | Lets the hit through, holds the body (Compressor VCA) |
| Vocal level | -22 dB, 3.5:1, 5 ms, 120 ms, knee 8, +6 dB, RMS | Opto | Levels a phrase; consonants still come through (Compressor Opto) |
| Smash | -28 dB, 12:1, 0.5 ms, 60 ms, knee 2, +16 dB | FET | All-buttons smash; under a lower Mix, parallel compression (Oldskool) |
| Sidechain pump | -30 dB, 10:1, 1 ms, 180 ms, keyed | Clean | Ducking from a kick |
| Opto bass | -24 dB, 4:1, 10 ms, 80 ms, knee 12, +10.5 dB | Opto | Even, round bass |
| FET vocal | -26 dB, 6:1, 1 ms, 90 ms, knee 2, +13.5 dB | FET | An aggressive, forward vocal |

Smash's threshold is -28 dB rather than the -35 first tried: matching the dry at -35 needed +22 dB of makeup, and a hit out of silence then passed its first half millisecond at +22 dB (peaks over +12 dBFS before the attack caught it); at -28 it needs +16 dB.

## 7. Measuring quality

`fxlab quality --class compressor` ([fx-quality.md](fx-quality.md), Compressor (K)) measures the defaults, each style (`compressor/opto`, `/bus`, `/fet`) and each preset. The measure itself had to be fixed first: fxlab read the gain on a level step once per 44 frames at 44.1 kHz (one millisecond at 44.1 kHz was taken as 44 frames, so the reading drifted), which made short times read long and a 1 ms attack read as exactly 1. The gain is now read at exact millisecond instants by demodulating the 1 kHz test tone through a 1 ms Hann window, with the 63 % and 90 % crossings refined by halving; `release_after` measures the release after a burst of a chosen length, for program-dependent releases.

| Id | What | Target |
|---|---|---|
| K1 | A steady 1 kHz sine from -39 to 0 dBFS against the drawn curve, Peak and RMS, knee 0 and 6 | Within 0.25 dB |
| K2 | The gain's 63 % time on a -40 to 0 dBFS step | Within ±15 % of Attack, 1 to 100 ms |
| K3 | The same back | Within ±10 % of Release, 50 ms to 2 s |
| K4 | THD of 50 and 100 Hz under 10 dB of reduction at the default times, Peak | ≤ -50 and ≤ -56 dB |
| K5, K6 | The meter; the linked quiet side | Within 0.1 dB |
| K7 | Loudness on pink at -18 dBFS: defaults with auto makeup, presets with their own and with auto | Within ±1.5 LU |
| K8 | Ratio 1, below the knee, Mix 0 | Bit for bit |
| K9 | Overshoot | Reported |
| K10 | Zipper on every control | ≤ 3 dB |
| K12 | The 90 % release after 3 s of 0 dBFS over after 50 ms | Clean 1 ± 0.1; the styles at least 2 |
| K13 | The curve at Range 6 | Within 0.25 dB |
| K14 | Unlinked: the quiet side; Link 50: half the loud side's reduction | Within 0.1 and 0.25 dB |
| K15 | Each style meets K1 to K4 (the release read after a burst, ±15 %; FET's colour reported) and K12; a style change ≤ 3 dB; FET's 5 kHz alias at 0 dBFS ≤ -60 dB | |

All 76 are met. Curve error 0.013 dB (Peak) and 0.026 (RMS), from 0.95 and 0.019; attack 1.08, 1.00, 1.00 and 1.00 times the label at 1, 10, 30 and 100 ms; release within 1 %; 50 Hz -57.7 dB and 100 Hz -69.3 dB THD at the defaults (Opto -62.1 at 50 Hz); Ratio 1 bit for bit; the presets -0.14 to +0.28 LU with their own makeup. The styles: Opto comes 90 % back in 333 ms after 50 ms of 0 dBFS and in 2.46 s after 3 s (7.4 times), Bus 386 ms and 0.90 s (2.3 times), FET 346 ms and 0.92 s (2.7 times), Clean the same either way; a style change reads 0 dB. FET's colour at 10 dB of reduction: -42.4 dB THD at 50 Hz, -43.0 at 100 Hz, -43.2 at 1 kHz; its 5 kHz alias -84.6 dB; an 8 kHz tone's third harmonic (24 kHz) folds to 20.1 kHz at -42.4 dB, reported.

Blessed worse, and why: the 1 ms attack reads 1.08 where it read exactly 1 (the old measure's artefact above; main's engine reads 1.58 with the fixed measure); RMS curve error 0.026 dB (was 0.019), inside 0.25: the hold now keeps the top of the RMS level's small 2 kHz ripple on a 1 kHz sine, where the old detector averaged it (the likely cause, not isolated); the zipper on Drum punch (0.49 dB, was 0.12) and Vocal level (0.64, was 0.20), inside 3 dB: those presets now reach their full drawn reduction and carry the Bus and Opto slow stages, so a sweep moves a larger gain.

Listening: a blind, loudness-matched A/B page of main against this PR goes with the PR.

## 8. Cost

The instruction gate's `Compressor` case (rustc 1.97.0, a 128-frame stereo block at 44.1 kHz, the defaults): 27.7k instructions, from 21.0k (1.32 times), about 0.8 % of a period on the MPC by round 7's fit (the Crush's 36.2k is about 1 %). The hold adds a second one-pole and a max per frame, Range a min, and the style's branches are per frame (the slow stage and FET's colour run only in their styles). Cases that carry it rise too: the drive, filter, delay and compressor chain 2.1 %, the mixer with a song's inserts (seven compressors among them) 3.8 to 5.7 %, the song 5.0 %. The Channel strip effect is built on this engine as well. The device round measures the real share; if it matters, the style's choices can become compile-time too, as the detector settings are.

## 9. Pitfalls and tests

| Pitfall | Guard |
|---|---|
| Less reduction than drawn | The decoupled detector on the reduction; K1; `static_curve`, `a_steady_tone_gets_the_reduction_drawn` |
| Times that don't mean what they say | Attack a time constant; `release_tau`; K2, K3; `attack_and_release_take_their_labels` |
| Distortion on bass | The hold keeps the gain still inside a cycle; K4 |
| Pumping, or a release too slow after hits | The styles' slow stage; K12; `the_styles_come_back_slower_after_long_reduction`, `opto_eases_in_and_still_takes_its_attack` |
| The kick pumping a mix | SC filter; `the_sc_filter_keeps_lows_from_pushing_the_detector` |
| Loudness bias | Auto makeup and level-matched presets; K7; `auto_makeup_adds_back_the_curve_at_a_mix_level` |
| Colour where it isn't wanted | FET only, scaled by the reduction; `fet_colours_only_as_it_turns_down` |
| Range or Link not doing what they say | K13, K14; `range_caps_the_reduction`, `link_sets_how_much_the_quiet_side_follows` |
| Cost when idle, or a changed sound when inserted | K8; `idle_it_passes_the_input_bit_for_bit` |
| Peak against RMS moving the threshold | RMS calibrated 3 dB up; `rms_and_peak_agree_on_a_steady_sine`, `rms_lets_short_hits_through` |
| Clicks on a style change | The reduction carried on, the colour glided; K15 |
| A block size changing the sound | `same_output_for_any_block_size` and the chain's block-size test, both with random automation |
| A preset that spikes out of silence | `every_preset_plays` (peaks under +12 dBFS on noise bursts from silence) |

## 10. History and next

- **Before round 2:** a branching peak detector on the raw signal with RMS as the other choice, the SC filter, the key, a parallel Mix, fast log and exp fits for the MPC's ARM.
- **Round 2, Compressor PR:** the smooth decoupled peak detector with `release_tau`; Style (Clean, Opto, Bus, FET), Auto makeup (off by default), Range and Link; presets retuned to their styles and to the dry's level, Opto bass and FET vocal added; fxlab's dynamics timing measure fixed. Saved projects' compressors (and every Channel strip's) change sound by the new detector: the full drawn reduction on steady material, attacks as labelled, less bass distortion. That is the fix, not a side effect.
- **Next:** look-ahead once a chain slot can change its latency (or as a Limiter-style always-on choice with a fixed latency); mid/side; hold; Pro-C-style "Dry first" parallel curve (plan-2 section 2.1); a level-over-time display; the device round's cost. The Limiter, Gate, de-esser, multiband and Ducker share most of these principles and can take the same detector.

## 11. Sources

- D. Giannoulis, M. Massberg and J. D. Reiss, "Digital dynamic range compressor design: a tutorial and analysis", JAES 60(6), 2012.
- THAT Corporation, "Attack and release time constants in RMS-based feedback compressors" II, AES preprint 4054.
- Universal Audio, "Ask the Doctors: analysis of dynamic range control devices"; 1176 Classic FET compressor manual.
- Minta Foundry, "The compressor bible" (LA-2A's two-stage release, SSL steps, settings by source).
- FabFilter, Pro-C 2 manual.
- J. C. Mason and D. C. Handscomb, "Chebyshev Polynomials", Chapman and Hall, 2003 (T2 and T3 turn a full-scale cosine into its second and third harmonics).
