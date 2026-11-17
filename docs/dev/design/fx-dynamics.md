<!--
covers: rust/mpfree-fx/src/limit.rs rust/mpfree-fx/src/master.rs rust/mpfree-fx/src/dynamics.rs rust/mpfree-fx/src/ducker.rs rust/mpfree-fx/src/transient.rs rust/mpfree-fx/src/strip.rs rust/mpfree-fxlab/src/quality/round4.rs
checked: 2026-11-17 1c113480
-->
# Dynamics: design

**Status:** 2026-11-17, FX quality round 4's Gate and Channel strip PR (after the Limiter and Master safety, Multiband and De-esser PRs). This page covers the dynamics effects other than the Compressor ([fx-compressor.md](fx-compressor.md)): the **Limiter**, **Master safety**, the **Gate**, the **De-esser**, the **Multiband comp**, the **Ducker**, the **Transient shaper** and the **Channel strip**. Round 4 improves them one pull request at a time, in the order of the plan (jferments/mpfree-claude `fx/quality/plan-4.md`, with research-dynamics.md and baseline-dynamics-2026-11-17.md): the Limiter and Master safety first, then the Multiband, the De-esser, the Gate with the Channel strip (this one), the Ducker and the Transient shaper. The sections on the effects not yet reworked say what they are today and what their pull request is planned to change. Their short entries are in [fx.md](fx.md); their measured targets are in [fx-quality.md](fx-quality.md) (ids P, N, S, X, U, T and C).

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

### Gate

Open and close thresholds apart (it shuts 4 dB under where it opens), Range, Attack (a straight ramp in gain), Hold, Release (an even fade in dB to Range), a 1 ms look-ahead, key filters and the strip's side-chain as the key.

- **Hold counts from the last peak.** The gate opens the frame the key reaches the threshold and shuts once the key hasn't reached the shut level (the threshold less 4 dB) for Hold, which is at least 20 ms (the fixed hold of about 20 to 30 ms Sound On Sound describes), so the dips between a bass note's peaks (12.5 ms apart at 40 Hz) never count. The old detector fell 60 dB in 20 ms between peaks: with Hold 0 it rode 40 and 60 Hz (3.7 and 2.3 dB of chatter inside a half cycle) and, under 1 kHz, shut only 0.2 to 1.1 dB under the threshold. Now it shuts within 0.37 to 0.61 dB of the threshold less 4 dB from 40 Hz to 1 kHz, Hold's worth of the falling tone late.
- **Range while shut.** A Range raised while the gate is shut lifts the gain to it over about 10 ms (a one-pole); a lowered one fades to it at Release's pace. It used to wait until the gate opened again.
- **Expander** (Mode, id 10; Ratio, id 11, 1.2 to 10, default 2): under the threshold the gain is (level - threshold) × (Ratio - 1) dB, at most down to Range, rising at Attack's pace and falling at Release's. Its level is the key's peak held for Hold (at least 20 ms); a lower peak within 1 dB takes over, so a fading note is followed peak by peak, and with none it falls 60 dB in 20 ms. Soft cleanup is an expander at Ratio 2 (its Range of -12 dB kept).
- **Listen** (id 9) plays the key as the filters leave it, to set them by ear.
- For the Channel strip: `feed` keeps the look-ahead filled while the strip has the gate off, and `resume_open` brings it back open (its key filters fresh), so switching it on is seamless.

### De-esser

- **Detection** listens above Freq (a 12 dB an octave Butterworth high-pass) on each side, and takes the louder, so an s panned to one side is caught as in the centre (it used to listen to the mid, where a one-sided s is 6 dB down: 4.4 dB less reduction in Absolute). **Relative** compares that band with the whole signal's louder side, so the threshold works at any level; **Absolute** is the band's own level. A 4:1 soft knee (4 dB) under Range; followers of 0.5 ms attack and 60 ms release.
- **Look-ahead** (decision 11.2): the sound is held back 2 ms (88 frames at 44.1 kHz, fixed, so the mixer compensates and the latency never moves) while the detector hears it as it comes in, so the gain is down when each s arrives. Before, the first half millisecond of each s passed.
- **Split**: an eighth-order Linkwitz-Riley split (a fourth-order Butterworth, squared; `kit::Split8`) a sixth of an octave under Freq (0.9 times). Its bands are in phase, so the low band plus the high band at the gain, `g × all-pass + (1 - g) × low`, is |low| + g |high| in level: never over 1, so nothing under the split is lifted, and it falls 48 dB an octave, so the cut is within a dB of Range from 1.5 times Freq up. The old split took a 12 dB an octave high-pass from the input (`x - high + g × high`), whose two halves weren't in phase: it lifted under Freq by up to 1.8 dB and fell 7 to 22 dB short of Range. Two Linkwitz-Riley orders were measured on paper first: a fourth-order split at Freq reaches Range only about two octaves up (8.7 dB short at Range 24), and an eighth-order one at Freq 2.1 dB short; the eighth-order at 0.9 times meets both targets with 1 dB to spare. At full reduction it takes about 2 dB off at 0.8 times Freq and nothing measurable at 0.6.
- **Wide** turns the whole signal down. It passes the split's all-pass too, as Split does at rest, so switching modes at rest doesn't click. **Listen** plays the high band (`all-pass - low`).
- Five state-variable stages a side make the split (one stage on the input gives the first low-pass and the all-pass's first half together), run on both sides at once (`Stage::run2`), which the compiler pairs up: 87.7k instructions a block run a side at a time, 59.0k together.

### Multiband comp

Two LR4 crossovers (Low | mid, 40 Hz to 1 kHz, 200 Hz; Mid | high, 1 to 12 kHz, 2.5 kHz, kept at least an octave over the lower) split the sound into three bands. The low band passes the upper crossover's all-pass, so the three sum to the input through both crossovers' all-passes: flat in level (within 0.06 dB), the phase turned around each crossover. The crossovers glide over 30 ms, re-tuned every 16 frames and landing on the frame the glide ends.

- **Mix in phase.** The dry is the bands' sum at unity, not the input, so each band's weight is `1 - Mix + Mix × gain` and any Mix adds two signals in phase. It used to add the input to the all-passed bands, which notched 19.7 dB at each crossover at Mix 50. Taking the dry from the bands needs no second set of all-passes. A band soloed out keeps only the dry's share.
- **The Compressor's detector in each band** ([fx-compressor.md](fx-compressor.md)): the louder side's peak in dB, the soft-knee curve (a 6 dB knee) for the reduction it asks for, a hold that jumps up at once and lets go at a release solved through the attack (`comp::release_tau`), then the attack's one-pole. Attack and Release are shared and mean their labels (63 %), and the curve is the one drawn. The old detector, a linear follower before the curve, fell 0.57 dB short of it and took 1.7 to 2.1 times Attack.
- **Thresholds and ratios glide** over 20 ms (the ratio as the curve's slope, 1 - 1/ratio), so turning them doesn't zipper.
- **Auto makeup** (new, id 19; off by default as the Compressor's, on in every preset): each band gets back what its curve takes at its share of a mix-level sound, pink noise at -18 dBFS RMS, read at -11 dBFS (about where the detector reads pink's peaks) less the band's share of the octaves from 20 Hz to 20 kHz (about a third each at the defaults, so near -15.8 dBFS). Gain adds on top. -11 is a dB under the Compressor's reference: at -10, Drum crush's 8:1 came out 1.7 LU loud.

What it can't do: at Attack 1 ms the mid band reaches 63 % in 1.44 ms of a step at 1 kHz. The band itself takes a few tenths of a millisecond to build up through the crossovers, and the detector can only act on what has arrived; only a look-ahead would hide it, and the multiband has no latency.

### Ducker (round 4's fifth PR)

Today: a volume envelope from the side-chain key or from every step of the tempo (song position while playing), with a straight attack, a hold and a shaped release. In tempo mode the duck starts at the beat rather than being down on it, a mid-step jump in song position loses its place, and two presets play away from their drawings. Planned (plan-4 section 8): a drawing that shows what plays, the attack before the beat (decision 11.3), key filters, dotted and triplet rates and a nudge.

### Transient shaper (round 4's last PR)

Today: a held peak and two pairs of dB followers (Attack: fast minus slow-rising; Sustain: slow-falling minus fast-falling) from a -60 dBFS floor, so a hit out of silence gets more than a hit over a quiet bed: its fronts' gain spreads 8.7 dB across levels. Planned (plan-4 section 9): a level-independent front (decision 11.4 on its default).

### Channel strip

MPFree's Gate, Compressor and EQ in one insert, with Order and Output. The gate and its fixes are the Gate's; the compressor is the Compressor engine.

- **Switches on an S-curve.** Each stage's switch is a straight 20 ms ramp heard through smoothstep (3t² - 2t³), so the fade starts and ends at rest. It was a one-pole, which starts at full speed: a 6 dB EQ boost switched off under a 220 Hz tone bent the wave 9.3 dB more than the tone itself does.
- **The gate comes back on open.** While the strip has the gate off, the gate's look-ahead keeps filling, and when it comes back on it starts open with its key filters fresh, then shuts over Hold and Release if the key is quiet. It used to start from an empty look-ahead, shut: a 1 ms gap, 12.9 dB over the tone's own bend.
- **Order dips.** An Order change fades the output to silence over 8 ms (smoothstep), swaps the stages there, stays silent while the gate's look-ahead (which still holds sound from the old order) empties and 2 ms more for the EQ's filters to settle on their new input, then comes back over 8 ms: about 20 ms in all. The fade counts frames, and the stages are run in a piece that ends where the fade reaches silence, so the swap lands on the same frame at any block size. The swap was instant (54.4 dB); with 5 ms fades the dip itself read 2.4 dB, with 8 ms 2.2, with 10 ms 1.8.
- **Style and Auto makeup** (ids 24 and 25, after the meters) are the compressor's own: Clean, Opto, Bus or FET, and Auto makeup off by default, as on the Compressor. Every preset turns Auto makeup on in place of its Makeup, and takes a style: Vocal and Bass Opto, Snare FET, Gentle glue Bus, Kick Clean.

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

**De-esser** (ids unchanged): Freq (2 to 12 kHz, 6k; log, 30 ms glide, the filters re-tuned every 16 frames), Threshold (-60 to 0 dB, -30), Range (0 to 24 dB, 10), Mode (Split, Wide), Detect (Relative, Absolute), Listen, and the Reduction meter. Latency 2 ms.

**Multiband comp** (ids unchanged, Auto makeup new): Low | mid (40 Hz to 1 kHz, 200; log, 30 ms glide), Mid | high (1 to 12 kHz, 2.5k; as Low | mid), per band Threshold (-60 to 0 dB, -18; 20 ms glide), Ratio (1 to 20, 2; log, its slope glides 20 ms) and Gain (-24 to +24 dB, 0; 20 ms glide), Attack (0.1 to 100 ms, 10; log), Release (10 ms to 1 s, 150; log), Solo (Off, Low, Mid, High), Mix (0 to 100 %, 100; 20 ms glide), Output (-24 to +24 dB, 0; 20 ms glide), Auto makeup (Off, On; Off), and the three bands' reduction meters.

**Gate** (ids 0 to 8 unchanged; Listen, Mode and Ratio new, appended): Threshold (-80 to 0 dB, -40; it shuts 4 dB under), Range (-80 to 0 dB, -80, silence at the bottom; a raised Range is reached over about 10 ms while shut), Attack (0.1 to 50 ms, 0.5; log), Hold (0 to 500 ms, 30; at least 20 ms in effect), Release (5 ms to 2 s, 120; log, an even fade in dB to Range), Key HP (20 Hz to 2 kHz, off at 20), Key LP (500 Hz to 20 kHz, off at the top), Key (Self, Sidechain), Listen (Off, On), Mode (Gate, Expander), Ratio (1.2 to 10, 2; log; Expander only), and the Gain meter. Latency 1 ms.

**Channel strip** (ids unchanged; Style and Auto makeup new, appended after the meters): the gate's Threshold, Range, Attack and Release; the compressor's Threshold, Ratio, Attack, Release, Makeup, Style (Clean, Opto, Bus, FET; Clean) and Auto makeup (Off, On; Off); the EQ's Low cut, Low freq and gain, Mid freq, gain and Q, High freq and gain; the three stage switches (20 ms on an S-curve), Order (an 8 ms dip either side of the swap), Output; the gate's gain and the compressor's reduction meters. Latency 1 ms, whatever is on.

**Master safety** (ids unchanged): Mode (Safe limiter, Soft clip, Raw; Safe limiter; a 10 ms glide between modes), Trim (-24 to +12 dB, 0; 20 ms glide), Ceiling (-6 to 0 dB, -0.3; 20 ms glide, new), Release (10 ms to 1 s, 80; log, as the Limiter's), and the meters Limiting, Overs and Clipped.

## 6. Defaults and presets

The Limiter's defaults (Gain 0, Ceiling -0.3, Release 80, Auto release on, True peak off) catch stray peaks without touching anything under the ceiling. Its presets: **Safe peaks** (Ceiling -1), **Loud master** (Gain 6, Ceiling -1, True peak on) and **Drum smash** (Gain 12, Release 25, Auto release off). Master safety's defaults (Safe limiter at -0.3 dB) are what every output ends in; it has no presets.

The De-esser's defaults (6 kHz, -30 dB Relative, Range 10, Split) and presets (Male vocal at 5 kHz, Female vocal at 7.5 kHz, both -26 dB; Harsh hats at 9 kHz, -20 dB, Range 6) are unchanged.

The Gate's defaults (Threshold -40, Range -80, Attack 0.5, Hold 30, Release 120) and presets (Tight drums, Gated snare, Kick trigger) are unchanged but for **Soft cleanup**, now an expander at Ratio 2 (Threshold -50, Range -12, Release 300): it turns what is under the threshold down gradually instead of switching to Range. The Channel strip's defaults are unchanged (the gate off, Clean, Auto makeup off, so saved projects sound as set); its presets now turn Auto makeup on in place of their own Makeup and each take a style: **Vocal** Opto, **Kick** Clean, **Snare** FET, **Bass** Opto, **Gentle glue** Bus.

The Multiband's defaults (200 Hz and 2.5 kHz, every band at -18 dB and 2:1, Attack 10 ms, Release 150 ms) are gentle glue. Its presets now all turn Auto makeup on: **Master glue**, **Tight low end** (keeps its +2 dB on the low band, a tilt), **Vocal smooth** and **Drum crush**, which drops the +6 dB it gave each band for Auto makeup (it was 5.6 LU quieter than the dry on pink, now 0.8 louder).

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

### Multiband

`fxlab quality --class multiband` measures the X targets ([fx-quality.md](fx-quality.md#multiband-compressor-x)). After the Multiband PR 31 of 32 are met (15 of 27 before; X7's five Auto makeup rows are new targets):

| | Before | After |
|---|---|---|
| Ratio 1 at Mix 25, 50 and 75: worst error | 5.9, 19.7, 5.9 dB | 0.06 dB |
| Mid band's curve at 1 kHz | 0.57 dB | 0.12 dB |
| Attack at 1, 10 and 30 ms, 63 % against the label | 2.08, 1.69, 1.69 | 1.44 (not met), 0.97, 0.94 |
| Release at 60, 150 and 500 ms | 0.85, 0.81, 0.81 | 1.00, 1.00, 1.00 |
| 50 Hz THD 12 dB over the low band's threshold | -50.0 dB | -61.7 dB |
| Zipper: mid Threshold, mid Ratio | 10.6, 6.8 dB | 0.05, 0.03 dB |
| Pink at -18 dBFS with Auto makeup: defaults, presets | | -0.3 LU; -0.3 to +0.8 LU |

The quick gates assert X2 at Mix 25, 50 and 75, and X7 at the defaults and at Drum crush's settings.

### De-esser

`fxlab quality --class deesser` measures the S targets ([fx-quality.md](fx-quality.md#de-esser-s)). After the De-esser PR all 20 are met (11 before):

| | Before | After |
|---|---|---|
| Split at full reduction: lift under 0.8 times Freq (Range 10, 24) | +1.3, +1.8 dB | +0.002 dB |
| Split at full reduction: worst distance from Range, 1.5 times Freq to 16 kHz (Range 10, 24) | 7.7, 22.1 dB | 0.10, 0.67 dB |
| An s on one side against the centre: Relative, Absolute | 0.70, 4.40 dB | 0.0003, 0 dB |
| Zipper: Freq, Range, Threshold | 0.09, 0.33, 0.61 dB | 0.25, 0.02, 0.02 dB |

S3 and S4 are as before (Relative takes the same 9.95 dB off the s at -6, -18 and -30 dBFS, and 0.04 dB off the voice between). `tests/coverage.rs` checks that the first millisecond of a burst out of silence already comes out turned down (the look-ahead), and `kit`'s unit tests that the split's all-pass is flat, that turning its high band down never lifts anything, and that an octave over the split it is within a dB of the gain.

### Gate and Channel strip

`fxlab quality --class gate` and `--class strip` measure the N and C targets ([fx-quality.md](fx-quality.md#gate-n)). After the Gate and Channel strip PR all 31 gate targets are met (20 of 30 before; N6, the expander, is new) and all 20 strip targets (10 of 14):

| | Before | After |
|---|---|---|
| Chatter with Hold 0: 40, 60, 100 Hz | 3.7, 2.3, 0.41 dB | 0 dB |
| Shut against the threshold less 4 dB, 40 Hz to 1 kHz, Hold 0 and 30 | 0.10 to 3.83 dB | 0.37 to 0.61 dB |
| Range from -80 to -40 dB while shut | 88 dB off (it didn't move) | 0.001 dB |
| The expander's curve, Ratio 2 and 4, 40 Hz and 1 kHz | | 0.02 dB |
| The strip's gate shut: 40 Hz, 1 kHz | 2.75, 0.10 dB | 0.75, 0.72 dB |
| The strip on pink at -18 dBFS with Auto makeup: defaults, presets | (off: -5.8, -8.0 to -1.8 LU) | -0.44, -0.77 to +0.65 LU |
| The strip's switches over a 220 Hz tone: gate on, compressor off, 6 dB EQ off, Order | 12.9, 0.30, 9.3, 54.4 dB | 0, 0.35, 0.43, 2.2 dB |

The fronts (N1), the release's label (N4) and the zippers (N5) are as before, all met. `tests/coverage.rs` checks that a 40 Hz note 2 dB over the threshold passes whole at Hold 0, that a Range moved while shut lands, that the expander turns a tone down by its ratio at 40 Hz and 1 kHz, and that Listen plays the filtered key; `strip.rs`'s tests that switching the gate on or changing Order doesn't bend a tone more than about 3 dB past its own.

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
| Multiband compressor (defaults) | 141.5k | 149.9k (+6.0 %) |
| De-esser (defaults) | 42.4k | 59.0k (+39.0 %) |
| Gate (threshold over the program's tone) | 17.4k | 17.3k (-0.8 %) |

The rises are the true peaks: Master safety now always catches them (decision 11.1). In the routing-only mixer cases a dozen loud tracks keep the master limiting nearly all the time, so the detector works out every point between samples for both channels on almost every frame. A mix that isn't pushing the master costs less than before (quiet blocks), and one near the ceiling pays for the near taps on most frames. The x86 host vectorises these sums poorly, so the numbers are a rough guide; on the MPC's ARM, which does floats one at a time, the folding and the near taps cut the detector's multiplies to 18 or 48 a channel from 96. Soft clip costs more than Safe limiter (the half-bands and four ADAA steps a frame); it isn't in the gate. The Multiband's rise is the detector in decibels and the six new glides; taking the dry from the bands kept the Mix fix free (a second pair of all-passes cost 22 %). The De-esser's rise is its split, five stages a side for one, and the look-ahead's delay. The Gate costs what it did (the bench's tone now sits just over its shut level, so it stays open, as it should); the expander's held peak and curve run only in Expander mode. The Channel strip has no bench case; with its gate off it now writes two samples a frame into the gate's look-ahead.

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
| A parallel Mix that notches a multiband's crossovers | The dry is the bands' own sum, in phase with them |
| Zipper from a band's threshold or ratio | Both glide; the curve takes the slope, so the ratio glides evenly |
| Auto makeup stacked on a preset's own makeup | The presets that turn it on drop their makeup gains |
| A split de-esser that boosts the voice under the s | A Linkwitz-Riley split, whose bands are in phase |
| A de-esser that misses the start of each s | 2 ms of look-ahead |
| An s panned to one side caught less | Detection from the louder side |
| A gate that flutters on bass | Hold counts from the last peak over the shut level and is at least 20 ms |
| A gate that shuts early on a fading note | The same: no falling detector between peaks |
| A control that does nothing while the gate is shut | A raised Range is reached over about 10 ms |
| A stage switched on with an empty look-ahead | The gate's look-ahead keeps filling while it is off, and it comes back open |
| A switch fade that starts at full speed | A straight ramp heard through smoothstep |
| A stage-order change that jumps | A dip to silence, the swap there, and a wait for the look-ahead and the EQ to settle |

## 10. History and next

- Before round 4: the Limiter's true-peak detector was a 12-tap Hann sinc at three phases (+0.48 dB over a 4x meter), its Auto release never acted, its release was a linear one-pole (63 % in dB at 0.8 of its label); Master safety had no true-peak detection (+0.81 dBTP out on pink pushed 12 dB) and a base-rate soft clip.
- This PR: the shared core, true peaks always caught in Master safety, the hold, release in dB, an Auto release that acts, the soft clip at 2x with ADAA, the mode glide, the Ceiling glide in Master safety.
- The De-esser PR: the eighth-order Linkwitz-Riley split under Freq, detection from the louder side, 2 ms of look-ahead (decision 11.2).
- The Multiband PR: Mix in phase, the Compressor's detector in each band, gliding thresholds and ratios, Auto makeup (on in every preset).
- The Gate and Channel strip PR: Hold from the last peak (at least 20 ms), Range while shut, the Expander mode and Key listen (plan-4 section 5.2's P2s), the Soft cleanup preset as an expander; the strip's S-curve switches, its gate back on open, the Order dip, Style and Auto makeup (on in every preset).
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
