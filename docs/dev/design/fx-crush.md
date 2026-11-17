<!--
covers: rust/mpfree-fx/src/crush.rs
checked: 2026-11-17 c87022f
-->
# Crush: design

**Status:** 2026-11-17, after FX quality round 2's Crush PR (#450). **Crush** (`mpfree.fx.crush`) lowers the bit depth and the sample rate, as an old sampler or a cheap converter does. Its short entry is in [fx.md](fx.md); its measured targets are in [fx-quality.md](fx-quality.md) (ids B1 to B10). The research notes and the plan this page draws on are in jferments/mpfree-claude `fx/quality/` (research-crush.md, plan-2.md section 5, `tools/crush_blep.py`). Perform FX's own crush and Lo-fi's grit are separate engines.

## 1. What it is for

Crush makes two kinds of damage on purpose. **Fewer bits** coarsen each sample's level: grit, noise and "granular" decays. **A lower rate** holds each sample for longer: the held staircase has images (copies of the sound mirrored around multiples of the rate) that fold into metallic, inharmonic tones and a dull or ringing top. The jobs:

| Job | Typical move | What matters most |
|---|---|---|
| Vintage sampler colour (SP-1200, S950, MPC60, Mirage, Emulator) | 12 bits at 26 to 40 kHz; 8 bits for the earliest machines; often on drums, in parallel | It sounds like those machines: their rate, their filters, their noise, not a broken plug-in |
| Grit on drums, bass, synths | 6 to 10 bits, 8 to 22 kHz, Mix 30 to 70 % | Grit that follows the music, no stray tones, quiet parts not gated off unless asked |
| Telephone, radio, toy, game console | 8 bits at 8 kHz; 4 to 6 bits for chiptune | The band-limited "phone" sound, which needs filters as much as crush |
| Destruction and sound design | 1 to 4 bits, a few hundred Hz to 4 kHz, swept | Wild but controllable; sweeps that don't click |

The MPC's Decimator opens as Crush's 12-bit classic preset and its Resampler as Soft grit (`mpc.rs`); AIR Lo-Fi opens Lo-fi, with Crush suggested for its bits.

## 2. Principles

**Bits.** Rounding to N bits adds an error of up to half a step. For a full-scale sine the signal against that error is 6.02 N + 1.76 dB (Bennett 1948), so the knob can be checked against theory. The error of a loud, busy signal is close to steady noise; the error of a quiet or simple one follows the signal and is heard as harsh distortion.

**Dither.** Adding a random value of up to one step, with a triangular distribution (TPDF, the sum of two uniform ones), before rounding makes the error independent of the signal: steady noise instead of distortion, 4.77 dB louder (Lipshitz, Wannamaker and Vanderkooy 1992).

**The quantiser's shape.** A mid-tread quantiser (rounding) has a level at zero, so anything under half a step comes out as silence: a reverb tail or a fade is cut off at -48 dBFS at 8 bits. A mid-riser has no level at zero, so a quiet tail becomes a buzz between the two levels nearest zero. Both are heard in real gear and both are choices.

**Rate.** A sample and hold at rate R turns a tone at f into lines at k R ± f, shaped by the hold's sin(x)/x (-3.9 dB at R/2). Those images, and the ones that land under the session's Nyquist after folding, are the effect.

**What isn't the effect.** A crusher running at the session's rate can only change its held value on a whole session frame. When R doesn't divide the session's rate, the hold lengths alternate (at 26.04 kHz in a 44.1 kHz session, between 1 and 2 frames) and the sampling instants jitter by up to a frame. That makes tones at neither f nor k R ± f, and the images above the session's Nyquist fold back too. No sampler being imitated makes them, and they change with the session's rate, so the same knob sounds different at 44.1 and 48 kHz. D16's Decimort says the same of its own design: "no internal aliasing, so only the modelled aliasing remains". The remedy is to sample the input at the exact fractional time, and to draw each step band-limited, so its timing between frames survives and nothing above the session's Nyquist is made.

**Filters.** Real samplers have a low-pass before the converter (anti-alias) and one after it (reconstruction), at or tied to their rate. Much of their sound, and most of the "phone" sound, is those band limits, not the bits.

## 3. Research

References studied (research-crush.md has the details and the sources): D16 Decimort 2 (anti-alias and image filters tied to the rate, jitter, mid-tread and mid-riser quantisers, dither; modelled on early Akai and E-mu samplers), Ableton Redux (Live 12's adds pre and post filters, jitter and a quantiser shape), XLN RC-20's Digital module, Logic's Bitcrusher (drive, clip modes), and the classic machines: the SP-1200 at 12-bit linear and 26.04 kHz, with "audible aliasing and imaging artifacts" from its pitch shifting and an analog filter on some outputs (Rossum's reissue page); the Akai S900 and S950 at 12 bits up to 40 and 48 kHz with sweepable anti-alias filters; the MPC60 at 12 bits and 40 kHz; the Ensoniq Mirage at 8 bits. Much of those machines' sound comes from pitching samples down without interpolation and from their analog filters, not from the bit depth alone (the research's judgement, consistent with Rossum's page and Decimort's design). Crush covers the converter; pitching down without interpolation belongs to the sampler.

What musicians expect, and where Crush stands:

| Feature | Common practice | Crush |
|---|---|---|
| Bits | 1 to 16 or 24, continuous | 1 to 16, continuous |
| Rate | Down to 100 to 200 Hz | 200 Hz to 48 kHz |
| Dither | Optional | TPDF, Off or On |
| Filters tied to the rate | Decimort, Redux, the S950 | Filters: Off, Before, After, Both |
| Jitter | Decimort, Redux | Jitter |
| Quantiser shape | Decimort, Redux | Tails: Cut (mid-tread) or Buzz (mid-riser) |
| Drive into the quantiser | Logic, RC-20 | Not yet (section 10) |
| Mix | All | Yes |

**The baseline** (main `5be7b53`, before the Crush PR): Bits honest within a dB of theory (-26.0, -49.7, -73.9 and -98.0 dB at 4, 8, 12 and 16 bits against -25.8, -49.9, -74.0 and -98.1); dither 4.6 to 6 dB, as TPDF should; rates that divide the session's clean (under -89 dB). But at rates that don't, 11 to 16 dB under the tone lay off the hold's grid (8 kHz -13.3 dB, the 12-bit classic preset's 26.04 kHz -13.4), at 8 bits a -50 dBFS sine came out as silence, and there were no filters, jitter or quantiser choice. A prototype (`tools/crush_blep.py`) showed the band-limited step does the work: exact sampling times alone left 8 kHz at -13.8 dB, a two-point polyBLEP got -26 to -34, a 32-frame table of band-limited steps about -50.

## 4. Our engine

Per side, at the session's rate:

```
in ─ [Before: 4th-order low-pass at 0.45 R] ─ sample at exact times (cubic) ─ + dither ─ quantise ─ band-limited step ─ [After: the same low-pass] ─ × Output ─┐
 └──────────────────────────────────────────── dry ─────────────────────────────────────────────────────────────────────────────────────────────────── Mix ─ out
```

**Exact timing** (decision 10.2, Jay 17 Nov: exact timing at every rate, no grid choice). A phase counter advances by R over the session's rate each frame. When it passes a hold, the time the hold fell due is known to a fraction of a frame; the input at that time is read by a cubic (Lagrange) through the last four frames, dithered if Dither is on, and quantised. A rate at or above the session's holds every frame, on the frame, and adds nothing else.

**The band-limited step.** The plain staircase would jump from the old held value to the new one at the next frame. Instead, the jump is spread as a minimum-phase band-limited step (minBLEP; Brandt 2001) placed at its exact time: a correction, the band-limited step less the plain one, is added into a 64-frame ring of corrections still to come. The step is built from a Kaiser-windowed sinc (beta 9, its edge at 0.9 of the session's Nyquist, 32 frames long, 16 points a frame), made minimum phase through its cepstrum (log magnitude, fold the cepstrum onto positive quefrencies, exponentiate) and summed into a step. The table `STEP` holds it with 17 phases (one per sixteenth of a frame, the last a frame on), read with linear interpolation between two phases; the test `the_step_is_what_its_recipe_makes` builds it again from the recipe and compares within 1e-6. Each hold that changes the value adds 32 corrections for each side (a hold at the same value adds none), in two straight runs round the ring.

Why minimum phase: a linear-phase step would ring before the step and need 16 frames of look-ahead, a latency the chain would have to pay. A minimum-phase step puts its ringing after, as an analog output filter does, so the latency stays 0 (XY FX relies on that). Turned down: exact times without band-limiting (-13.8 dB at 8 kHz); a two-point polyBLEP (-26 to -34 dB); a nearest-entry step table (the prototype's -27 dB floor at 32 kHz came from it, hence interpolation between phases); oversampling the whole hold (it would not fix the timing and costs far more).

**Quantiser.** At Bits b the step is 2^(1 - b) of full scale (Bits glides, so it is continuous; the level count is worked out only while it moves, since `exp2` per sample is a library call on the MPC's ARM). **Tails Cut** rounds (mid-tread): under half a step is silence, as most crushers. **Tails Buzz** floors and adds half a step (mid-riser): no level at zero, so a quiet tail buzzes between the two levels nearest zero (a -50 dBFS sine at 8 bits comes out at -48 dBFS), and an input of exactly zero stays zero, so digital silence stays silent.

**Dither** is TPDF of ±1 step, from a xorshift generator reseeded on reset (so a render is repeatable).

**Filters** (Off, Before, After, Both): fourth-order Butterworth low-passes (two state-variable stages, Q 0.541 and 1.307) at 0.45 times the rate, or of the session's rate if that is lower. Before is the anti-alias filter in front of the converter; After is the reconstruction filter behind it. Off in the plain default, on in the vintage presets (decision 10.2). A change fades each one in or out over 10 ms (a filter faded out stops running and starts again from rest); while Rate glides their corners follow it every 16 frames on a grid counted from reset, so any block size sounds the same. `Crush::filters_response_db` gives their curve for the page's Filters card.

**Jitter** moves each hold by up to half its length at random (100 %), for a wobbly clock, from a second generator so turning it doesn't change the dither's sequence.

**Output** (-24 to +12 dB) trims the wet, so a filtered preset can play as loud as the dry.

## 5. Controls

| Control | Range (default) | Law and smoothing |
|---|---|---|
| Bits | 1 to 16 (8) | Linear, continuous, 20 ms glide |
| Rate | 200 Hz to 48 kHz (11,025 Hz) | Log, 20 ms glide; Filters follow it every 16 frames |
| Dither | Off, On (Off) | Toggle |
| Tails | Cut, Buzz (Cut) | Choice |
| Filters | Off, Before, After, Both (Off) | Each fades over 10 ms |
| Jitter | 0 to 100 % (0) | Linear |
| Output | -24 to +12 dB (0) | 20 ms glide |
| Mix | 0 to 100 % (100) | Linear, 20 ms glide |

Bits and Rate are the page's heroes; Dither, Tails and Filters are rows; the cards draw a sine through it and the Filters' curve. Ids 0 to 3 are the first version's; 4 to 7 were appended by the Crush PR, so saved projects load and keep their settings.

## 6. Defaults and presets

**The default**, 8 bits at 11,025 Hz with Filters off, is plainly crushed but still musical, and a rate that divides 44.1 kHz. Filters are off in it so the plain effect stays the raw converter (decision 10.2).

**Presets**, each within 1 LU of the dry on pink at -18 dBFS (Output sets the filtered ones' level):

| Preset | Settings | Why |
|---|---|---|
| 12-bit classic | 12 bits, 26.04 kHz, Filters Both, +1 dB | The SP-1200's converter (the MPC's Decimator) |
| 12-bit 40k | 12 bits, 40 kHz, Filters Both | The MPC60's |
| 8-bit sampler | 8 bits, 22,050 Hz, Filters Both, +1.5 dB | An early 8-bit sampler (the Mirage era) |
| Soft grit | 10 bits, 32 kHz, dither | A subtle texture (the MPC's Resampler) |
| Chip | 6 bits, 22,050 Hz | Chiptune grit |
| Phone line | 8 bits, 8 kHz, dither, Filters Both, +3 dB | A phone or radio: the band ends where a phone line's does (3.6 kHz) |
| Wobbly clock | 8 bits, 6 kHz, Jitter 40, Tails Buzz | An unstable clock with tails that buzz |
| Destroyed | 3 bits, 2 kHz | Sound design |

## 7. Measuring quality

`fxlab quality --class crush` ([fx-quality.md](fx-quality.md), Crush (B)), with a measure written for it (`sampling`): the spectrum through a four-term Blackman-Harris window (a rate that doesn't divide the session's puts lines between bins, and a plain spectrum's leakage read as stray energy; the first baseline was corrected for this), then everything that is not at k R ± f, against the tone.

| Id | What | Target |
|---|---|---|
| B1 | Noise and distortion of a full-scale sine, 4 to 16 bits | Within 1 dB of 6.02 N + 1.76 dB |
| B2 | Dither's noise rise; a -40 dBFS sine at 8 bits with dither | 4 to 6 dB; THD ≤ -30 dB |
| B3 | What lies off the hold's grid at 8, 10, 11.025, 22.05, 26.04 and 32 kHz, in 44.1 and 48 kHz sessions | ≤ -45 dB |
| B4 | The images at a quarter of the rate | Within 1 dB of sin(x)/x |
| B6 | A -50 dBFS sine at 8 bits | Cut reported; Buzz ≥ -60 dBFS |
| B7 | The Filters card against a sine through them | Within 0.5 dB |
| B8 | A Bits or Rate change's step against the crushed signal's own | ≤ 3 dB over it (fxlab's usual zipper compares with the resting ends, which a staircase defeats; reported) |
| B9 | Loudness, defaults and presets | Within ±1 LU |
| B10 | Silence | ≤ -120 dBFS |

After the Crush PR all 40 are met (19 of 32 before). Off the grid now -74.3, -54.6, -91.4, -92.3, -64.0 and -68.3 dB at the six rates in a 44.1 kHz session and -90.0 to -60.7 at 48 kHz; the worst is where the step's own transition band lets an image fold near the top (10 kHz at 44.1). The images follow sin(x)/x within 0.0015 dB; Buzz keeps -50 dBFS at -48.2; the Filters' drawing is within 0.0012 dB of what plays. The presets' off-grid row runs with Jitter at 0, since jitter moves the holds off any grid on purpose.

Listening: the Crush PR's blind, loudness-matched A/B page (main against the PR, five cases: plain settings at 10 kHz, the 12-bit classic and Phone line presets, Rate falling from 16 kHz to 1 kHz, and the Destroyed preset) is https://claude.ai/artifact/HntxAfeZNDSX3ZTWsabokt.

## 8. Cost

The instruction gate's `Crush` case (the default 8 bits at 11,025 Hz; rustc 1.97.0, a 128-frame stereo block at 44.1 kHz): 36.2k instructions, 3.5 times the old 10.3k, about 1 % of a period on the MPC by round 7's fit. Most of it is the 32 corrections each changed hold adds; reading them from the 17-phase table in two straight runs round the ring halved a first version's 71.3k. The lever if the device round finds it too dear: a 24-frame step, at a few dB of the off-grid margin (crush-device-checks.md in project files).

## 9. Pitfalls and tests

| Pitfall | Guard |
|---|---|
| Stray tones from the frame grid; a sound that changes with the session's rate | Exact times and band-limited steps; B3 at both session rates; `each_hold_takes_the_input_at_its_exact_time` |
| A step table that isn't what it claims | `the_step_is_what_its_recipe_makes`, `the_step_starts_at_zero_and_ends_at_one` |
| The interpolation | `the_cubic_passes_through_its_points_and_lines` |
| Tails cut off | Tails Buzz; B6 |
| Noise from silence (dither on) | `silence_stays_silent_with_either_tails`; B10; the rest rules |
| Clicks on Filters changes or Rate sweeps | 10 ms fades; corners on a 16-frame grid; `turning_the_filters_on_and_off_does_not_click`; B8 |
| A block size changing the sound | The 16-frame grid counted from reset; `same_output_for_any_block_size` in `tests/fx.rs` |
| Unfair A/B | Output on the filtered presets; B9 |

## 10. History and next

- **Before round 2:** bits and rate with optional dither, held on the session's frame grid, mid-tread only, no filters.
- **Round 2, Crush PR (#450, 17 Nov):** exact timing with a minimum-phase band-limited step (no latency), Tails, Filters, Jitter and Output, three presets (12-bit 40k, 8-bit sampler, Wobbly clock), a Filters card.
- **Next (plan-2 section 5.2, P2):** **Drive** into the quantiser (0 to 24 dB, level compensated), so a quiet source hits more of the steps; non-linear (companding) quantiser shapes, if the MPC60's reported non-linear encoding is confirmed. The device round decides whether the 32-frame step stays.

## 11. Sources

- W. R. Bennett, "Spectra of quantized signals", Bell System Technical Journal 27, 1948.
- S. P. Lipshitz, R. A. Wannamaker and J. Vanderkooy, "Quantization and dither: a theoretical survey", JAES 40(5), 1992.
- E. Brandt, "Hard sync without aliasing", ICMC 2001 (minBLEP); V. Välimäki and colleagues on BLEP and polyBLEP.
- Rossum Electro-Music, SP-1200 education page; D16 Group, Decimort 2; Attack Magazine, "Character crunch: replicating the sound of vintage samplers"; XLN Audio, RC-20 Retro Color manual; Ableton, Live 12 audio effect reference (Redux).
