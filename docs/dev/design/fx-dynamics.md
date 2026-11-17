<!--
covers: rust/mpfree-fx/src/limit.rs rust/mpfree-fx/src/master.rs rust/mpfree-fx/src/dynamics.rs rust/mpfree-fx/src/ducker.rs rust/mpfree-fx/src/transient.rs rust/mpfree-fx/src/strip.rs rust/mpfree-fxlab/src/quality/round4.rs
checked: 2026-11-17 1c113480
-->
# Dynamics: design

**Status:** 2026-11-17, FX quality round 4's Limiter and Master safety PR. This page covers the dynamics effects other than the Compressor ([fx-compressor.md](fx-compressor.md)): the **Limiter**, **Master safety**, the **Gate**, the **De-esser**, the **Multiband comp**, the **Ducker**, the **Transient shaper** and the **Channel strip**. Round 4 improves them one pull request at a time, in the order of the plan (jferments/mpfree-claude `fx/quality/plan-4.md`, with research-dynamics.md and baseline-dynamics-2026-11-17.md): the Limiter and Master safety first (this one), then the Multiband, the De-esser, the Gate with the Channel strip, the Ducker and the Transient shaper. The sections on the effects not yet reworked say what they are today and what their pull request is planned to change. Their short entries are in [fx.md](fx.md); their measured targets are in [fx-quality.md](fx-quality.md) (ids P, N, S, X, U, T and C).

## 1. What they are for

| Effect | Jobs | What matters most |
|---|---|---|
| **Limiter** | Catching peaks on a drum bus or a hot track (2 to 6 dB, fast release); master loudness (3 to 8 dB, program-dependent release, ceiling -1 to -0.3 dBTP); delivery (a true-peak ceiling about -1 dBTP) | Nothing over the ceiling, in the samples and in the wave a converter rebuilds between them; no distortion on bass; no pumping; the punch kept |
| **Master safety** | The last stage of every output, rarely touched | Nothing reaches the converter clipped, and nothing audible while the level is fine |
| **Gate** | Stopping drums from ringing, cleaning up noise between phrases, gated snare and trigger effects | Fronts kept, no chatter, no buzz on bass |
| **De-esser** | Turning down harsh "s" and "t" sounds on a voice; taming hats | Only the sibilance moves; the voice's body and level stay |
| **Multiband comp** | Mastering glue, a tight low end, a smooth vocal, crushed drums | Flat when idle, and at any Mix |
| **Ducker** | The pump: on each kick, or on every step of the tempo | Down on the beat, the curve that is drawn |
| **Transient shaper** | More or less punch and ring on drums | The same change at any level, nothing on steady tones |
| **Channel strip** | A gate, a compressor and an EQ in one insert, as on a desk | Each stage as good as the effect it comes from; switching without clicks |

The MPC's AIR Maximizer and AIR Limiter open the Limiter, AIR Noise Gate the Gate, AIR Pumper and Mother Ducker the Ducker, AIR Channel Strip the Channel strip (`mpc.rs`).

## 2. Principles

**The wave between the samples.** A converter rebuilds a smooth wave through the samples, and it can go over the highest sample: iZotope gives 3 dB as an everyday example and 7.43 dB in a pathological case at 44.1 kHz. Fast limiting, hard clipping and lossy encoding make these inter-sample (true) peaks. BS.1770 reads them by oversampling at least four times; FabFilter Pro-L 2 in its true-peak mode keeps them within 0.1 dB. A limiter that holds only its samples at the ceiling can still clip the converter.

**Look ahead, at a fixed latency.** A brick wall must be down before the peak arrives, so it delays the audio and works out the gain from what is coming. The delay must not change with any setting: the mixer compensates a fixed latency, and an export fails if a latency moves mid-pass.

**Only ever lower the gain.** Each stage after the detector (hold, release, ramp) may take the gain lower than what the frame's peak needs, never higher, so the ceiling holds by construction and no clamp has to cut corners into the wave.

**Gain riding inside a cycle is distortion.** A limiter that lets go between the crests of a bass note moves the gain inside each cycle: gain-modulation distortion. A hold that keeps the gain still from one crest to the next fixes it (the LazyLimiter design: a fade down before each peak, a hold while any coming sample needs as much, then a release; no distortion on sines into deep bass). Universal Audio cite about -43 dB as a common audibility line.

**Times mean their labels**, by the convention the compressor uses since round 2: a time constant, 63 % of the way in decibels.

**Program-dependent release.** Fast after a short peak, so there is no hole after a snare; slow under a sustained push, so a dense mix doesn't pump.

**Clipping that doesn't fold back.** A clipper makes harmonics; at the base rate those above half the sample rate fold back as inharmonic tones, and the clipped wave's true peaks go over its samples. Running the curve at a higher rate with antiderivative antialiasing (the Drive's ADAA, round 2) keeps the harmonics where they belong.

## 3. Research

The references (research-dynamics.md has the details): FabFilter Pro-L 2 (eight program-dependent styles, look-ahead, attack and release, channel linking for transients and release, true-peak limiting at 4x within 0.1 dB, loudness metering), iZotope on true peaks, BS.1770's 4x true-peak meter, magnetophon's LazyLimiter (fade, hold and logarithmic release), Pro-G and Sound On Sound on gates (hysteresis, a hold of 20 to 30 ms against bass cycles, range, key filters), Pro-DS (wide and split, look-ahead), Pro-MB (phase modes), SPL's Transient Designer (level independence), Kickstart and LFO Tool (drawn ducks), and the round 2 compressor research ([fx-compressor.md](fx-compressor.md)) on detectors and distortion.

What musicians expect of a limiter, and where MPFree's stand:

| Feature | Common practice | Limiter | Master safety |
|---|---|---|---|
| Gain (input drive), ceiling, release | All | Gain 0 to 24 dB, Ceiling -12 to 0 dB, Release 5 ms to 1 s | Trim -24 to +12 dB, Ceiling -6 to 0 dB, Release 10 ms to 1 s |
| True-peak limiting | Pro-L 2, most mastering limiters | True peak (off by default) | Always (decision 11.1) |
| Program-dependent release | Pro-L 2's styles, "auto" | Auto release (on by default) | No: a fixed release |
| Look-ahead | Adjustable, 0 to 5 ms | Fixed 1.5 ms | Fixed 1.5 ms |
| Styles, channel link | Pro-L 2 | Not yet (plan-4 section 4.2, item 6) | |
| Clip modes | Clippers, "soft clip" stages | | Soft clip, Raw |

## 4. The engines

### The shared limiter (`limit.rs`)

The Limiter and Master safety run the same core, `PeakLimit`. Per frame, for both channels together (linked):

```
in ─┬─────────────────────────────────────────────────────────── delay (window + 15 frames) ──────── × gain ─ out
    └─ detect (sample, or 4x true peak) ─ need = ceiling / peak ─ smallest over window + hold ─ release (dB) ─ ramp (box over window) ─┘
```

1. **Detect.** The sample 16 frames back (`TP_LAG`), and with true peaks on, the three points a quarter, a half and three quarters of the way to the next sample. Each point is a 32-tap Kaiser-windowed sinc (β 4) over the 16 samples on each side, normalised to unity at DC: within 0.08 dB of a sine's level up to 19 kHz at 44.1 kHz, over rather than under at 20 kHz, and no shorter than fxlab's 48-tap meter at 20.5 kHz. A β of 6 is flatter (0.01 dB) but reads 0.3 dB short at 20 kHz, where a soft-clipped master has energy; with it Master safety's Soft clip passed 0.11 dB over its ceiling. The phases are **folded** about the window's centre: the half-way phase is symmetric and the three-quarter phase is the quarter phase backwards, so with `s` and `d` the sums and differences of samples paired from both ends, the three points are `even·s + odd·d`, `even·s - odd·d` and `mid·s`: 48 multiplies a channel instead of 96 (the MPC's ARM runs floats one at a time, so multiplies are what count there). Two exact shortcuts keep most frames cheaper, and neither changes a need:
   - **Quiet windows.** While no sample in the window is over 0.38 of the ceiling (one over the largest sum of a phase's tap sizes, 2.63), no point between samples can pass the ceiling, and the detector skips them.
   - **Near taps first.** The 12 taps nearest the points come first (18 multiplies). The other 20 add at most 0.31 of the window's loudest sample, so while the window's samples are known to be under 0.7, 1 or 1.4 of the ceiling (a counter for each, frames since a sample passed it) and the near taps' points leave that much room under the ceiling, no point can pass it, and the far taps are skipped. A ceiling that moves counts every sample as loud until the window has passed.
2. **Hold.** The need (the ceiling over the peak) goes into a sliding minimum over the look-ahead window plus a hold, a quarter of Release up to 5 ms. The gain then barely moves between the crests of a bass note: 50 Hz under 6 dB of limiting distorts at -54 dB at the defaults (-30 before). A 10 ms hold keeps it still (-134 dB) but costs 0.3 LU more on a pushed mix (section 7), and 5 ms is already far under the audibility line.
3. **Release, in decibels.** Down at once, back up as a one-pole on the gain in dB. The hold and the ramp's half window come before it, so its time constant is Release less those (at least a quarter of Release), and the gain is 63 % of the way back in decibels at Release. With **Auto release** the fast stage aims at a third of Release, and a slow stage follows the need over half a second and lets go at three times Release; the gain comes back no faster than the slow stage, so after a short peak it is back in a third of Release and after a long push in about three times it. The fast path skips the conversions: a frame that needs nothing and has nothing to let go returns 1; the need's dB is worked out again only when it moves; while the gain is down at the need (attack and hold) it is the need itself, without an exponential.
4. **Ramp.** The released gain averaged over the look-ahead window (a running sum in 64-bit floats), so it falls in a smooth ramp and is all the way down when the peak leaves.

Every stage takes the gain at most down to what is needed, never up past it, so at the frame a peak leaves every gain in the box is at or under its need and the output is at the ceiling or under. The audio leaves `window + 15` frames late: 81 at 44.1 kHz, 87 at 48 kHz, whatever the settings.

**Quiet blocks.** Most of the time nothing is near the ceiling. When the core is at rest (no loud sample in the detector's window, no need held, both release stages back at 0 dB and the box all ones) and every sample of a block is under the quiet share of a steady ceiling, every frame's gain would be exactly 1, so `quiet_block` only delays the block and moves the history and counters on: the same numbers bit for bit, at a fraction of the cost. The Limiter and Master safety try it first whenever their gain and ceiling aren't gliding.

### Limiter

`Limiter` (`dynamics.rs`): Gain (gliding over 20 ms) into the core, at Ceiling (gliding over 20 ms), with Release, Auto release and True peak passed on. The output is clamped at the ceiling, which only acts while the ceiling glides down; otherwise the gain is already low enough. The Reduction meter is the deepest gain of the block. Its rest before sleep (fx.md) is ten of its slowest stage, plus the hold and the latency.

### Master safety

`MasterSafety` (`master.rs`): Trim (gliding), a 5 Hz DC blocker, then one of three modes, with the core's true peaks always on (decision 11.1):

- **Safe limiter** (default): the core at the ceiling. Neither the samples nor the 4x true peaks pass it.
- **Soft clip**: a curve that is straight to half the ceiling and bends in to the ceiling along `tanh`, run at twice the rate: Drive's Eco half-bands (five all-pass sections, flat to 20 kHz and 72 dB down from 25 kHz at 2x of 44.1 kHz), first-order ADAA with the curve's antiderivative (`knee·(|x| - knee/2) + span²·ln cosh((|x| - knee)/span)`), and the `Flat2x` filter that restores the top the averaging takes. Then the same core at the same ceiling catches the little the band-limited curve still overshoots, so Soft clip is colour and still never over.
- **Raw**: no limiting; what is over 0 dBFS clips in the converter.

The soft clip's delay (the half-bands' 2.02 frames at low frequencies and ADAA's 2) is matched in the other modes by a 4-frame pad, so the latency is the core's plus 4 in every mode: 85 frames at 44.1 kHz, 91 at 48 kHz. A mode change glides over 10 ms: the soft clip's share of what the core hears and the core's share of the output each move in a straight line, both paths running meanwhile, and the soft clip starts from a clean state when it fades in from nothing. The meters count overs (frames that came in above the ceiling), frames that leave above 0 dBFS (only Raw can make them) and the limiting.

### Gate (round 4's Gate PR is next but two)

Today: open and close thresholds apart (hysteresis), Range, Attack, Hold, Release, a 1 ms look-ahead, key filters and the strip's side-chain as the key. Its fronts are kept exactly and the fade means its label, but with Hold 0 it rides 40 and 60 Hz, it shuts only 0.2 to 1.1 dB under the threshold under 1 kHz, and a Range move while it is shut waits until it opens. Planned (plan-4 section 5): a detector that holds each peak for at least 20 ms, Range that follows while shut, an expander mode and Key listen.

### De-esser (round 4's third PR)

Today: listens above Freq; Relative detection compares that band with the whole sound, so its threshold works at any level; Split turns down only the band (a complementary split), Wide the whole sound; Listen plays the band. Split lifts the band under Freq by up to 1.8 dB and falls short of Range above it. Planned (plan-4 section 6): a split that never lifts (a Linkwitz-Riley split or a matched shelf, whichever measures better), linked detection, and look-ahead (decision 11.2).

### Multiband comp (round 4's second PR)

Today: two LR4 crossovers make three bands that sum flat at Mix 100, each with threshold, ratio and gain. Mix 50 notches 19.7 dB at a crossover (the dry and the bands' phase disagree), and the band detectors are not the compressor's. Planned (plan-4 section 7): the dry through the same all-passes so any Mix adds in phase, the compressor's detector in every band, and auto makeup.

### Ducker (round 4's fifth PR)

Today: a volume envelope from the side-chain key or from every step of the tempo (song position while playing), with a straight attack, a hold and a shaped release. In tempo mode the duck starts at the beat rather than being down on it, a mid-step jump in song position loses its place, and two presets play away from their drawings. Planned (plan-4 section 8): a drawing that shows what plays, the attack before the beat (decision 11.3), key filters, dotted and triplet rates and a nudge.

### Transient shaper (round 4's last PR)

Today: a held peak and two pairs of dB followers (Attack: fast minus slow-rising; Sustain: slow-falling minus fast-falling) from a -60 dBFS floor, so a hit out of silence gets more than a hit over a quiet bed: its fronts' gain spreads 8.7 dB across levels. Planned (plan-4 section 9): a level-independent front (decision 11.4 on its default).

### Channel strip (with the Gate's PR)

Today: MPFree's Gate, Compressor (Clean) and EQ in one insert, with Order and Output. Switching its gate on clicks (it restarts with an empty look-ahead), a boosted EQ switched off clicks (the fade starts at full speed), and an Order change has no crossfade. Planned (plan-4 section 9): click-free switches and order changes, and the Gate's fixes.

## 5. Controls

**Limiter** (ids unchanged):

| Control | Range (default) | Law and smoothing |
|---|---|---|
| Gain | 0 to 24 dB (0) | 20 ms glide |
| Ceiling | -12 to 0 dB (-0.3) | 20 ms glide |
| Release | 5 ms to 1 s (80) | Log; 63 % back in dB, after a hold of a quarter of it (at most 5 ms) |
| Auto release | Off, On (On) | A third of Release after a short peak, three times it after a long push |
| True peak | Off, On (Off) | The points between samples as a 4x meter reads them |
| Reduction | 0 to 30 dB, read only | The meter |

**Master safety** (ids unchanged): Mode (Safe limiter, Soft clip, Raw; Safe limiter; a 10 ms glide between modes), Trim (-24 to +12 dB, 0; 20 ms glide), Ceiling (-6 to 0 dB, -0.3; 20 ms glide, new), Release (10 ms to 1 s, 80; log, as the Limiter's), and the meters Limiting, Overs and Clipped.

## 6. Defaults and presets

The Limiter's defaults (Gain 0, Ceiling -0.3, Release 80, Auto release on, True peak off) catch stray peaks without touching anything under the ceiling. Its presets: **Safe peaks** (Ceiling -1), **Loud master** (Gain 6, Ceiling -1, True peak on) and **Drum smash** (Gain 12, Release 25, Auto release off). Master safety's defaults (Safe limiter at -0.3 dB) are what every output ends in; it has no presets.

## 7. Measuring quality

`fxlab quality --class limiter` measures the P targets ([fx-quality.md](fx-quality.md#limiter-and-master-safety-p)) on the Limiter at its defaults, with True peak, with Auto release off, at each preset, and on Master safety's Safe limiter and Soft clip. After this PR all 33 are met (17 before). The headline numbers, pushed up to 12 dB on pink and on a programme with drums and bass:

| | Before | After |
|---|---|---|
| True peaks over the ceiling: Limiter with True peak, Master safety, Soft clip | +0.48, +1.11, +4.37 dB | +0.003, +0.0004, +0.002 dB |
| 50 Hz THD under 6 dB of limiting: Limiter, Master safety | -29.8, -38.7 dB | -53.9, -63.1 dB |
| Auto release: 63 % back after 0.1 s and after 3 s | 21.8 and 21.8 ms | 26.4 and 242.8 ms |
| Release with Auto off (Master safety), 63 % against the label at 20, 80 and 300 ms | 0.83, 0.80, 0.80 (0.80) | 0.99 (0.99 to 1.00) |
| Soft clip: 5 kHz pushed 6 and 12 dB, worst alias | -27.6, -17.0 dB | -81.8, -62.3 dB |
| Latency (44.1 kHz): Limiter, Master safety | 71, 66 frames | 81, 85 frames |

**The price in loudness**, reported: the programme pushed 6 dB keeps 2.61 LU of the push through the Limiter at its defaults (3.21 before), 2.40 with True peak (3.08), 1.80 with Auto release off (2.47) and 1.86 through Master safety (2.54). The old engines were louder because they rode the gain inside the bass's cycles and let go early; the hold accounts for about 0.35 LU, the release that now means its label for the rest. A blind level-matched listening page goes with the pull request.

The quick gates (`mpfree-fxlab/tests/gates.rs`) assert P1 and P2 on pink pushed 12 dB for the Limiter and for both of Master safety's limiting modes, P4 on the Limiter's defaults, and P7 across settings and modes; `mpfree-fx`'s unit tests check the detector's phases against a sine and the folded form against the full one, that a quiet window can skip the points between samples, that the far taps never take a point past the room the near taps leave (on random windows and on the worst signs), that a quiet block comes out bit for bit as its frames would, and that a mode switch steps no further than the sine it plays.

## 8. Cost

The instruction gate (rustc 1.97.0, a 128-frame block at 44.1 kHz), before and after:

| Case | Before | After |
|---|---|---|
| Limiter (12 dB of push) | 39.5k | 44.9k (+13.5 %) |
| Limiter (true peak) | 72.6k | 98.3k (+35.3 %) |
| Master safety | 29.0k | 30.3k (+4.7 %) |
| Mixer, idle (63 strips, nothing playing) | 52.8k | 44.8k (-15.2 %) |
| Mixer, 12 tracks and 8 pads, routing only | 210.8k | 308.5k (+46.3 %) |
| Mixer, 16 tracks and 32 pads, routing only | 370.8k | 461.7k (+24.5 %) |
| Mixer cases with a song's inserts | 1.02M to 1.54M | +5 to +7 % |

The rises are the true peaks: Master safety now always catches them (decision 11.1). In the routing-only mixer cases a dozen loud tracks keep the master limiting nearly all the time, so the detector works out every point between samples for both channels on almost every frame. A mix that isn't pushing the master costs less than before (quiet blocks), and one near the ceiling pays for the near taps on most frames. The x86 host vectorises these sums poorly, so the numbers are a rough guide; on the MPC's ARM, which does floats one at a time, the folding and the near taps cut the detector's multiplies to 18 or 48 a channel from 96. Soft clip costs more than Safe limiter (the half-bands and four ADAA steps a frame); it isn't in the gate.

## 9. Pitfalls and how the design avoids them

| Pitfall | How it is avoided |
|---|---|
| Inter-sample overs from a "safe" limiter | True-peak detection matched to a 4x meter, always on in Master safety |
| A clamp that cuts corners into the wave (and makes overs of its own) | The gain is low enough by construction; the clamp at the ceiling only acts while the ceiling glides down |
| Buzz on bass from a fast release | The hold, a quarter of Release up to 5 ms |
| An "auto" release that never acts | The slow stage caps the recovery (it used to take the larger of it and what the input needs, which was 1 during every release) |
| A release that isn't its label | A time constant in dB, with the hold and the ramp taken out |
| A soft clip that aliases and goes over | 2x with ADAA, then the same limiter |
| A latency that moves with a setting | The detector's lag and the soft clip's pad are there in every mode and setting |
| Clicks on a mode change | A 10 ms glide with both paths running |
| Denormals in the half-bands | Their states are flushed every 32 frames, on a grid from the stream's start |

## 10. History and next

- Before round 4: the Limiter's true-peak detector was a 12-tap Hann sinc at three phases (+0.48 dB over a 4x meter), its Auto release never acted, its release was a linear one-pole (63 % in dB at 0.8 of its label); Master safety had no true-peak detection (+0.81 dBTP out on pink pushed 12 dB) and a base-rate soft clip.
- This PR: the shared core, true peaks always caught in Master safety, the hold, release in dB, an Auto release that acts, the soft clip at 2x with ADAA, the mode glide, the Ceiling glide in Master safety.
- Next, from plan-4 section 4.2: styles in the spirit of Pro-L 2's (Transparent, Punchy with a shorter hold, Safe with a longer one) and a channel link amount, if the device round shows the cost allows. Then the rest of round 4 as section 4 says.

## 11. Sources

- FabFilter, Pro-L 2 help: [advanced settings](https://www.fabfilter.com/help/pro-l/using/advancedsettings), [true peak limiting](https://www.fabfilter.com/help/pro-l/using/truepeaklimiting).
- iZotope, [What is a true peak limiter?](https://www.izotope.com/en/learn/true-peak-limiter).
- ITU-R BS.1770, the 4x true-peak meter (as iZotope describes it).
- magnetophon, [LazyLimiter](https://github.com/magnetophon/LazyLimiter).
- Parker, Zavalishin and Le Bihan, "Reducing the aliasing of nonlinear waveshaping using continuous-time convolution", DAFx 2016 (ADAA; see `adaa.rs`).
- Valenzuela and Constantinides, the polyphase IIR half-band (see `oversample.rs`).
- Sound On Sound, [Advanced gating techniques, part 1](https://www.soundonsound.com/techniques/advanced-gating-techniques-part-1); FabFilter [Pro-DS](https://www.fabfilter.com/help/pro-ds/using/advancedcontrols) and [Pro-MB](https://www.fabfilter.com/help/pro-mb/using/processingmode) help; Sound On Sound's [SPL Transient Designer review](https://www.soundonsound.com/node/4924915).
- Giannoulis, Massberg and Reiss, "Digital Dynamic Range Compressor Design: A Tutorial and Analysis", JAES 2012 (via [fx-compressor.md](fx-compressor.md)).
