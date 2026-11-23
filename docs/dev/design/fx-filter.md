<!--
covers: rust/mpfree-fx/src/svf.rs rust/mpfree-filter rust/mpfree-sampler/src/dsp.rs rust/mpfree-sampler/src/engine/render.rs
checked: 2026-11-23 1349659
-->
# Filters: design

**Status:** 2026-11-17, after FX quality round 2's Filters PR and its Ladder and Res mode PR, which added the Multi filter's **Model** (Multi or **Ladder**) and **Res mode** (Soft, Hard or Clean: what holds the resonance back). The **Multi filter** (`mpfree.fx.filter`) and the **DJ filter** (`mpfree.fx.dj_filter`) are the Filter family: one effect in the picker with two modes. Their short entries are in [fx.md](fx.md); their measured targets are in [fx-quality.md](fx-quality.md) (ids F1 to F12). The research notes and the plan this page draws on are in jferments/mpfree-claude `fx/quality/` (research-filter.md, plan-2.md section 7). The EQ is built on the same state-variable filter (its design is in fx.md's EQ entry); the Auto wah, the Comb and the XY FX filter are separate engines. From 2026-11-23 this page is also the design page for the **pad filter** (each pad's and keygroup's filter in `mpfree-sampler`), which the filter gap plan brings up to the effect's standard and past it, toward what stock MPC OS 3.10.1 offers: [The pad filter](#the-pad-filter) below.

## 1. What it is for

| Job | Typical move | What matters most |
|---|---|---|
| Sweeps and builds (a low-pass opening over 8 bars, the DJ filter) | One knob or automation across most of the range, some resonance | Smooth at any speed; the level stays sensible through the sweep |
| Acid and synth-style resonance on bass lines and loops | Low-pass 12 or 24 dB, high resonance, cutoff 200 Hz to 3 kHz, moved; often drive into it | A peak that sings without blowing up the level |
| Thinning and band-limiting (radio, phone, a high-pass before a reverb) | High-pass 100 to 500 Hz, band-pass 300 Hz to 3 kHz | Accurate corners and slopes |
| Tone shaping (darker, warmer, less fizz) | A gentle low-pass at 2 to 8 kHz, shelves | A flat passband, nothing added |
| Drive and grit | Drive into a resonant low-pass | Grit without fizz |

The MPC's HP Filter, LP Filter and AIR Filter open as the Multi filter (`mpc.rs`).

## 2. Principles

**The state-variable filter.** Andrew Simper's trapezoidal ("zero-delay feedback") SVF: two integrators in a loop, solved exactly each sample, with the cutoff pre-warped, so the -3 dB point of a Butterworth low-pass is at the cutoff at any frequency below Nyquist and the filter stays stable however fast the cutoff moves (Zavalishin, *The Art of VA Filter Design*). Its band-pass state `v1` is what a resonance builds up: at the cutoff it is Q times the input, and the low-pass and high-pass outputs follow it.

**Resonance in analog filters saturates.** A linear filter's peak grows with Q without limit: at Q 40 a loud sound at the cutoff comes out 32 dB over itself, and a mix-level loop swept through it clips. In an analog filter the resonance loop runs into the limits of its transistors or op-amps, so the peak compresses as the signal grows and the filter rings up fully only on quiet material (the Moog ladder's tanh stages, Huovilainen DAFx 2004). Bounding the loop's state does the same in a digital filter. Some analog filters clip their resonance hard instead, and that is part of their sound: the Korg MS-20's diodes in its resonance loop give it a harder, buzzier scream. And sometimes no ceiling at all is what is wanted: a clean, deep ring that a quiet part can't trip over, the way MPFree's filter was before this round. So the ceiling is a choice, the **Res mode** (Jay's pick on decision 10.4).

**A switch is a jump in the output.** A filter's types share nothing in what they put out at a given moment: a low-pass and a high-pass of the same sound differ by the whole sound, so switching between them on a frame is a step as big as the signal. Crossfading the two outputs over a few milliseconds removes it.

**Drive aliases.** A saturator before the filter makes harmonics; those above Nyquist fold back as inharmonic tones. Running it at twice the rate between half-band filters, with first-order antiderivative antialiasing (ADAA), takes most of them out (the Drive PR's tools, [fx-drive.md](fx-drive.md)).

## 3. Research

The full notes are in research-filter.md. In short: musicians expect the cutoff to mean the frequency, a resonance that sings but doesn't blow up, smooth sweeps and switches, true slopes, clean drive, and on analog-styled filters self-oscillation in tune. Ableton's Auto Filter, FabFilter Volcano, Kilohearts Filter and the analog filters people know (Moog ladder, Oberheim SEM, MS-20, TB-303) were the references. Measured before this round: the cutoffs were exact (0.02 %), the passband flat, Resonance-0 sweeps clean; the resonance linear and unbounded (pink at a mix level through a swept low-pass at full Resonance came out at +2.8 dBFS), a type change clicked (64 dB over the sound) and the drive aliased at -20 dB.

## 4. Our engine

### Multi filter

```
in ─┬─ plain delay (4 frames) ─────────────────────────────┐
    └─ up 2x ─ soft curve, ADAA, flat FIR ─ down ─ (wet)    ├─ blend ─ stage 1 ─ stage 2 (24 dB) ─ out
       the half-bands' phase + 2 frames ─ (dry, matched) ───┘
```

**The stages.** One SVF for 12 dB, two for 24 dB. At 24 dB a low-pass or high-pass is a Butterworth-shaped first stage (Q 0.541) and a second stage carrying the resonance (Q 1.307 times the set Q over 0.707), so Resonance 0 is a true fourth-order Butterworth; a band-pass or notch at 24 dB is two equal stages. Resonance 0 to 100 % maps to Q 0.707 to 40 exponentially. The bell and shelves have one stage at either slope.

**Bounded resonance** (Res mode **Soft**, the default). In the resonant stages (a low-pass's or high-pass's only stage, the second at 24 dB, and both of a band-pass's) the band-pass state is bent as it is worked out: unchanged up to a knee, then `knee + room · u / (1 + u)` with `u` how far past the knee it is over `room`, so it approaches `knee + room` smoothly and never passes it. The low-pass state integrates the bent band-pass (`v2 = ic2 + g · v1`), so the whole loop is bounded, not just the output. At Resonance 25 % and up the knee is 0.35 and the room 0.3 (full scale 1); below 25 % both grow as 1 over (Resonance / 25 %), so a gentle filter never bends a mix-level sound; at Resonance 0 nothing bends and the stages are the plain linear SVF, bit for bit. The notch, bell and shelves don't ring up at their output and stay linear. The result: a -40 dBFS sine at the cutoff still rings up 31 dB at full Resonance (32 dB linear), while pink at -18 dBFS RMS through a swept low-pass at full Resonance peaks at -2.0 dBFS (12 dB) and -3.1 (24 dB), from +2.8 and +1.0.

**Res mode** (Soft, Hard, Clean; Soft by default, so a project saved before it sounds as it did) picks what bounds the resonant stages' band-pass state:

- **Soft** is the bend above, at the session rate.
- **Hard** clips it instead: straight up to three quarters of a ceiling, flat from five quarters, a parabola between (the slope from 1 to 0, no corner, as a diode's edge), the ceiling 0.6 of full scale at Resonance 25 % and up and rising below as Soft's knee does (none at Resonance 0). The low-pass state integrates the clipped value, as with the bend. Because a clip adds more harmonics than the bend, the Hard filter runs at twice the rate between the same half-bands as the drive (its own stages, designed at the doubled rate, which is also what the drawing uses), so a loud sine at a full-Resonance high-pass's cutoff aliases at -82.9 dB where Soft at the session rate is at -57.5. A loud sound's ring is flatter-topped and buzzier, about as loud at its peak (swept pink at full Resonance -1.6 and -1.9 dBFS), and a quiet one rings up fully (32.0 dB), since the clip is straight up to its edge.
- **Clean** has no ceiling: the stages are the plain linear SVF (the knee is infinite), as before this round, so a loud sound at the cutoff rings up the full 32 dB and can go over full scale (swept pink at full Resonance +2.8 and +1.0 dBFS).

Soft and Clean are the same engine: a change between them moves the bend's knee and room smoothly to infinity or back (they are divided by a value that ramps between 1 and 0 over 10 ms), so it never clicks. Hard is a second engine, so a change to or from it crossfades the two over 10 ms; the one coming in starts from the other's band-pass and low-pass states, which are nearly the same at either rate (an SVF's states are those signals themselves), rather than from silence, so a long ring carries on through the change. A type or slope change on Hard crossfades as on Soft.

Chosen while building: bending the band-pass state rather than the input or the output (bending the input leaves the peak unbounded; bending the output flattens the passband of a loud sound and leaves the loop's state to grow); a rational knee rather than `tanh` (no transcendental per sample, and exactly linear below the knee, so quiet material and Resonance 0 are untouched); the knee scaled down below 25 % Resonance rather than one knee for all (a fixed knee bent loud drums through a flat low-pass).

**Type and slope changes** crossfade over 10 ms with a smoothstep. The old filter keeps running from its state while the new one takes over; the new one starts from the old one's state (the 12 dB filter's types share it), and a second stage newly in use starts from silence. A change in the first half of a fade fades from the same old filter; later, from the one that was coming in, so a fast run of changes never jumps.

**Drive** (0 to 24 dB) runs at twice the rate: Drive's Eco half-bands (polyphase IIR, five all-pass sections, flat to 0.19 of the doubled rate and 71 dB down from 0.31), the soft curve `x(27 + x²)/(27 + 9x²)` averaged by first-order ADAA (its antiderivative worked out in 64 bits), then the short flat FIR that makes ADAA's two-point mean flat to 20 kHz. It blends in over the first 3 dB, against a dry side taken through the half-bands' phase and the same delay, so the two add in phase. With the drive off the sound takes a plain 4-frame delay instead, and when the drive comes in or leaves the dry side moves between the plain delay and the matched one over 256 frames. So the **latency is a fixed 4 frames** (reported for compensation; it was 0), the same with the drive on or off, and the drive can be automated without the sound moving in time. The half-bands' states are flushed of values too small to hear every 64 frames on a grid counted from reset.

**At rest** (Soft or Clean, no glide, no fade, the drive off: most of the time) the delay and each stage run a block at a time with their state in registers, the same numbers as frame by frame; the clean filter costs less than before this PR (see section 8).

**Glides.** Cutoff (in the log domain), Resonance, Gain and Drive glide over about 10 ms, the coefficients worked out again each frame while they move.

**The drawing** (`MultiFilter::response_db`) is the linear filter's response (on Hard, of the stages at twice the rate), exact for quiet material; a loud sound at a high Resonance rings less than drawn, by design.

### The Ladder model

**Model** (Multi, Ladder; Multi by default) picks the filter. **Ladder** is a Moog-style 24 dB low-pass: four one-pole low-passes in a loop, the loop's input sum through a curve (on Soft `tanh`, the transistor pair at the bottom of a real ladder), solved without a delay in the loop (Zavalishin's TPT ladder: the loop is solved linearly each sample and `tanh` is taken of that solution, so it stays stable at any setting and in tune up to the top), run at twice the rate between the same half-bands as the drive.

```
in ─ up 2x ─ (drive: soft curve, ADAA, flat FIR, blended in over 3 dB) ─ tanh(in − k · out) ─ 4 one-poles ─ × makeup ─ down ─ out
                                                                          └──────────── feedback k ─────────────┘
```

- **Resonance** is the feedback `k`, 0 to 4.3 in a straight line (3.9 on Clean). At 4 the loop gains 1 at the cutoff and it sings on its own, so from about 93 % up (on Soft and Hard) the ladder self-oscillates, a sine at the cutoff (within 0.06 cents from 100 Hz to 5 kHz) whose level `tanh` holds steady (-10.8 dBFS RMS, the same within 0.01 dB across that range). Below that the peak grows as an analog ladder's does, a little under the cutoff at low Resonance.
- **The bass loss.** A ladder's passband falls as `1 / (1 + k)` as Resonance rises (14 dB at the edge of oscillation), which is part of its sound. The output makeup `1 + k / 2` gives back about two thirds, so at full Resonance the low end is 4.7 dB down instead of 14.
- **Its own saturation.** `tanh` at the loop's input bounds the resonance (swept pink at full Resonance peaks at -2.2 dBFS) and colours loud material; at a mix level it is nearly straight.
- **Res mode** on the ladder picks that curve. **Soft** is `tanh`. **Hard** is the same rounded clip as the Multi model's, its ceiling 0.5, where `tanh`'s knee is: a mix-level sound passes about as through `tanh`, the ring sings about as loud (-10.6 dBFS RMS, in tune within 0.24 cents) and swept pink peaks at -3.6 dBFS, but a loud sound clips flat where `tanh` would still be rounding. **Clean** is straight, with the feedback at full Resonance 3.9 instead of 4.3: with nothing to hold it a ladder past 4 would grow without end, so Clean rings long (about a quarter of a second at 100 Hz) but never sings, and a loud sound at its peak can go over full scale (swept pink +0.7 dBFS). A change of Res mode on the ladder blends the two curves and the feedback over 10 ms (a smoothstep; turned back during a fade it carries on from the same blend), so a singing ladder fades out or in rather than stopping.
- **Drive** goes into the ladder the same way as into the clean filter (the soft curve, at 2x with ADAA). **Type**, **Slope** and **Gain** don't apply, and the page dims them ("the ladder is a 24 dB low-pass").
- **A Model change** crossfades over 10 ms, the model coming in starting from silence (1.2 dB over the sound, against 3); turned back during a fade, it carries on from the same blend. Each of the three engines (Multi at the session rate for Soft and Clean, Multi at twice it for Hard, the ladder) has its own level that rises over 10 ms while it is the one set and falls while it isn't, so any run of Model and Res mode changes fades without a jump. The ladder's path has the clean path's 4 frames of latency.
- **Rest.** From Resonance 93 % up a self-oscillating ladder never goes quiet on its own, so the Multi filter says it never rests (`rest_frames` and `tail_frames` are `u32::MAX` there) and a chain never sleeps it mid-note; below that its tail follows the slowest poles' decay.
- **Drawn as heard:** `response_db` gives the linear ladder's response at twice the rate (each one-pole `G (1 + z^-1) / (1 + (2G - 1) z^-1)`, the loop `H^4 / (1 + k H^4)`, the makeup), within 0.002 dB of a quiet sine through it.

Chosen while building: the TPT ladder with `tanh` on the linear solution rather than Huovilainen's model with a `tanh` in every stage (that one needs its half-sample delay compensated and costs four `tanh` a sample; one at the input gives the bounded resonance and the steady self-oscillation that matter most); the makeup at two thirds rather than none (the ladder's full bass loss made a resonant ladder much quieter than the clean filter at the same settings) or full (no bass loss at all is not a ladder); Resonance in a straight line (a curve bunched the singing range into the last few percent).

### The pad filter

Each pad, and each keygroup, has one filter after its voices' read (`mpfree-sampler` `src/dsp.rs`, run per voice in `src/engine/render.rs`): **Low-pass**, **High-pass**, **Band-pass** and **Notch** at 2 or 4 poles, the **Ladder** (with **Drive** up to 24 dB at twice the rate), the **Vowel** (two formants in parallel, the cutoff picking the vowel) and the **Comb** (a tuned delay and sum). Its cutoff moves by the filter envelope (`env_octaves` at full level), velocity (`vel_to_cutoff`), the pad's LFO (up to four octaves either way) and pressure, recomputed every 16 frames (`FILTER_STEP`).

What it is today, measured by the sampler ratchet's PF rows ([fx-quality.md](fx-quality.md#the-sampler)): the state-variable stages are the effect's trapezoidal SVF, but each at Q 0.5 at Resonance 0 (`k = 2 - 1.95 × resonance`, Q 0.5 to 20) and a 4-pole is two equal stages (the second at half the resonance). So the cutoff isn't the -3 dB point (a 4-pole low-pass at 1 kHz is 3 dB down at 435 Hz), the response sags 3 to 9 dB from a Butterworth around the cutoff, and the resonance is linear and unbounded with a lower top than the effect's (swept pink at full Resonance peaks at +0.2 dBFS through the 4-pole; a quiet sine rings up 26 dB, against the effect's 31). The Ladder is the effect's kind of TPT ladder with a `tanh`-like curve on the feedback, which sings in tune (DSP quality's Y2b), but with Drive its loop runs at the base rate after the drive's 2x, which aliases near the top (PF7).

The filter gap plan (jferments/mpfree-claude `fx/quality/filters-mpc-gap/`, revised on 23 Nov after Jay's design review; step by step under [History and next](#10-history-and-next)) closes this in thirteen phases: the architecture below (F0), the measures (F1, done), the shared crate and the clean fixes (F2: Butterworth stages, a resonance policy per model, a per-frame cutoff, the filter before the amp and the driven Ladder inside the 2x), modulation through `mpfree-mod` (F3), more slopes and a Peak (F4), the first full filter UI (F5), the Ladder rebuilt (F6), two filters per pad (F7), Smooth and Scream (F8), voices (F9), Diode and Classic 12 (F10), the auto filter (F11) and device performance (F12, with a cost check in every phase).

### The shared filter system (design, step F0)

What the filter gap plan builds towards; the parts built so far are marked. Every filter, in the pads, the Filter effect and later the synths, is one crate, [`mpfree-filter`](../../../rust/mpfree-filter), in four layers (logical parts of the one crate, with plain enums rather than a plugin framework):

1. **DSP algorithms:** SVF stages (built: the effect's stage and its bounded resonance, moved from `mpfree-fx` bit for bit), Butterworth cascades, ladders, the diode solve, Sallen-Key, formant banks, comb and half-bands. They take physical values (Hz, Q, gain) and know nothing of pads, tracks or screens.
2. **Models and parameters:** what a musician picks. Each model declares its parameters (name, range, unit, curve, default), its **cutoff meaning** (the -3 dB point at neutral resonance for a Butterworth low-pass or high-pass, a nominal frequency as resonance rises, the centre of a band-pass, notch or bell, a documented reference frequency for an analog model), its **resonance policy** (neutral, musical and maximum resonance, whether it self-oscillates, its gain and limiting: Clean, Saturated or Self-oscillating in spirit, but each model its own), its **stability contract** (the range it is proven or tested stable in, what glides and what steps, behaviour at the extremes, finite-state checks and a reset to silence if a state ever goes non-finite), its solver (the cheapest that passes its stress tests: closed form, fixed-cost linearisation, a fixed few Newton steps; the [reference](#7-measuring-quality) is the yardstick) and its oversampling (linear types at the base rate; nonlinear ones 2x or 4x, by measurement; no stateful ADAA inside a feedback loop).
3. **Instances and state:** integrators, feedback, half-band state, smoothing, cached coefficients and crossfades. A configuration is shared; an instance belongs to one sounding hit, or to one channel of the Filter effect.
4. **Hosts:** the sampler (one chain per hit), the Filter effect (one chain per channel, always running) and later the synths.

**Parameter flow.** `mpfree-mod` resolves base, morph, macro, lock, touch and matrix into one value per parameter, as it does for everything ([modulation.md](modulation.md)). The host adds the voice-local sources in octaves (filter envelope, velocity, keytrack from the voice's tuned pitch `key_cents`, a random offset seeded from the project and the event, pressure and the pad LFO) and clamps the sum to the model's range; the instance smooths it every frame. Nothing else decides what a parameter means. The pad's Cutoff and Resonance are already addressable offsets in octaves that glide on playing voices (`mpfree-sampler` `src/node.rs`).

**Gain staging.** A nonlinear model is input Drive, then its core, then an output trim, so more Drive means more harmonics without much more level. The reference level is -18 dBFS RMS pink (the lab's and the Drive effect's), and Drive is in dB above it. No automatic loudness matching: fixed calibration between implementations and a model's own compensation (a ladder's bass loss) only.

**A hit's signal path.** The layers a hit plays are read and mixed with their own gain and pan; the mix passes Filter A, then Filter B, then the amp envelope, then the pad's level and pan. A hit whose layers are all mono is filtered in mono. Today it is different: each layer is its own voice with its own filter, after the amp envelope (`render.rs` `finish`), so the grouping is a step of F2, agreed with the sampler refactor. The Comb runs after the amp so it rings on after the sound ends; a self-oscillating model needs no input and stops with its hit.

**Changes while notes play.** Cutoff, Resonance, Drive and envelope depth glide on sounding hits. A change of model, slope or routing crossfades over 10 ms, old and new side by side, its cost counted in the worst case. Envelope times reach each envelope at its next stage. **Bypass** crossfades over 10 ms; a cheap model runs on while bypassed, an oversampled one stops after the fade and restarts from silence. A stolen or choked hit fades with its amp and its state is cleared before the instance plays again.

**Identity, presets and versions.** Filter A and B are stable ids whatever their order, and each parameter's address keeps its meaning across models (a model without Drive ignores it). A filter configuration can be copied, pasted and saved on its own; an instrument preset holds it. Saves carry a filter version from F2, and an older one is mapped onto the new filters with a one-time notice.

### DJ filter

One knob: below the middle a low-pass falling from 20 kHz to 80 Hz, above it a high-pass rising from 20 Hz to 8 kHz, one SVF stage with Q from 0.707 to 6.7 by Resonance. The filter fades in over the first 10 % each side of the middle, and in the middle it is off and the input passes bit for bit, its state cleared so the next turn starts clean. Perform FX runs it inside its own loop (`DjFilter::frame`). Its peak is low enough (Q 6.7) that swept pink at full Resonance peaks at -3.9 dBFS, so it has no bounded resonance; this round changed nothing in it.

## 5. Controls

| Control | Range (default) | Law and smoothing |
|---|---|---|
| Type | Low-pass, High-pass, Band-pass, Notch, Bell, Low shelf, High shelf (Low-pass) | 10 ms crossfade |
| Slope | 12 dB, 24 dB (12 dB) | 10 ms crossfade; the bell and shelves have one slope |
| Cutoff | 20 Hz to 20 kHz (20 kHz) | Log; 10 ms glide in the log domain |
| Resonance | 0 to 100 % (0) | Q 0.707 to 40, exponential; 10 ms glide |
| Gain | -24 to +24 dB (0) | The bell's and shelves'; 10 ms glide |
| Drive | 0 to 24 dB (0) | Blends in over the first 3 dB; 10 ms glide |
| Model | Multi, Ladder (Multi) | 10 ms crossfade; Ladder dims Type, Slope and Gain |
| Res mode | Soft, Hard, Clean (Soft) | 10 ms: Soft to Clean moves the knee, Hard crossfades, the ladder blends its curves |
| DJ: Position | -100 to +100 (0) | Low-pass left, high-pass right, off in the middle; 15 ms glide |
| DJ: Resonance | 0 to 100 % | Q 0.707 to 6.7; 15 ms glide |

## 6. Defaults and presets

**The default** is an open 12 dB low-pass at 20 kHz with no resonance or drive: it passes the sound, 4 frames late, so adding it changes nothing until the cutoff moves.

**Presets** (`presets.rs`), unchanged by this PR: Warm low-pass (24 dB at 2.5 kHz, Resonance 20), Acid squelch (24 dB at 900 Hz, Resonance 80, Drive 9 dB), Thin high-pass (300 Hz, Resonance 10), Radio band (band-pass at 1.5 kHz, Resonance 30, Drive 6 dB), Hollow notch (24 dB at 1 kHz), Sub lift (a low shelf) and, new, Ladder bass (the ladder at 500 Hz, Resonance 70, Drive 6 dB: a round, resonant synth-bass filter) and Screaming low-pass (12 dB at 1.2 kHz, Resonance 85, Drive 6 dB, Res mode Hard: a lead or bass filter that screams when pushed). Acid squelch and Radio band now drive at twice the rate, and Acid squelch's resonance bends on loud notes instead of clipping.

## 7. Measuring quality

`fxlab quality --class filter` measures the Multi filter at both slopes and the DJ filter ([fx-quality.md](fx-quality.md), Filters (F)):

| Id | What | Target |
|---|---|---|
| F1 | The drawn -3 dB point against Cutoff, 100 Hz to 16 kHz; heard against drawn | Within 1 %; within 0.1 dB |
| F2 | 100 Hz under a 1 kHz low-pass at Resonance 0, 50, 100 | Within ±0.5 dB |
| F3 | Pink at -18 dBFS RMS through a low-pass swept 100 Hz to 10 kHz at Resonance 100 | Peak ≤ -1 dBFS |
| F4 | A -40 dBFS sine at the cutoff, Resonance 100 | Rings up ≥ 28 dB |
| F5 | Low-pass to high-pass at 24 dB, and 12 to 24 dB, under a tone | ≤ 3 dB over the sound |
| F6 | Sweeps end to end in 50 ms at Resonance 0 | ≤ 3 dB (slope zipper); Resonance 50 reported |
| F7 | Drive 12 dB, a 5 kHz sine at 0 dBFS: worst alias | ≤ -65 dB |
| F8 | DJ: bit for bit off in the middle, sweeps, corners, swept pink | As F6 and F3; low-pass to 100 Hz or lower, high-pass from 5 kHz or higher |
| F9 | The ladder at Resonance 100 after a click: pitch against the cutoff at 100 Hz to 5 kHz, and the level's spread | Within 5 cents; within 1 dB |
| F10 | Zipper on every preset | Reported |
| F12 | The Res mode: Hard's swept pink peak, quiet ring and drawing; Soft to Hard and to Clean on both models; the Hard ladder singing in tune and in bounds; the Clean ladder never singing | ≤ -1 dBFS; ≥ 28 dB; ≤ 0.1 dB; ≤ 3 dB; within 5 cents and ≤ -1 dBFS; ≤ -100 dBFS after a click |
| F11 | Cost | The clean filter ≤ 1.2 times before; the drive states its own |

All 52 are met (30 for the Multi filter on Soft and the DJ filter, 9 for the ladder, 13 for the Res mode): swept pink peaks at -2.0 and -3.1 dBFS (were +2.8 and +1.0); a quiet sine rings up 31.1 dB; a type change reads 0 dB and a slope change 1.2 (were 63.5 and 29.3); the drive's alias is -71.3 dB at Drive 12 (was -19.7; -41.1 at Drive 24, reported); the cutoffs, passband, sweeps and the DJ filter are as they were. The ladder sings within 0.06 cents of the cutoff from 100 Hz to 5 kHz at a level steady within 0.01 dB (F9), swept pink peaks at -2.2 dBFS (F3), it is heard within 0.002 dB of its drawing (F1) and a Model change reads 1.2 dB (F5); on Hard swept pink peaks at -1.6 and -1.9 dBFS and a quiet sine rings up 32.0 dB, the Hard ladder sings within 0.24 cents and swept pink through it peaks at -3.6 dBFS, the Clean ladder's click rings out to silence, and no Res mode change reads over 0.5 dB (F12); reported: a quiet sine at Resonance 90 rings up 24.2 dB, the passband at full Resonance is -4.7 dB, and a 5 kHz sine at 0 dBFS aliases at -114.9 dB with no drive and -52.4 dB at Drive 12 (the drive's curve is antialiased, the ladder's `tanh` after it is not; the Amp's and Fuzz's targets for heavy distortion are -50). `tests/filter.rs` checks that with the drive off the filter is the linear one exactly, 4 frames late, at any Resonance 0 setting and for the notch and bell; that a loud sound bends and a quiet one rings up; that a type or slope change makes no step bigger than the tone's own; that the ladder sings in tune at a steady level, plays as drawn, holds a loud sound and rings up a quiet one, and crossfades a Model change (there and back within a fade); that Clean is the linear filter at any level, Hard clips a loud sound and rings up a quiet one, a Res mode change crossfades on both models (back within a fade too), and the ladder sings on Hard and rings out on Clean; and that any block size sounds the same through a change of type, slope, Res mode, model and drive.

Listening: a blind, loudness-matched page (main against this PR: Acid squelch swept on a bass line, a full-Resonance sweep over a loop, low-pass to high-pass and 12 to 24 dB on drums, drive into a low-pass on a lead, Warm low-pass on keys, which should sound the same, and the Ladder bass against main's 24 dB low-pass at the same settings) is https://claude.ai/artifact/Wa7nNrn45aHqJ1iw12TSVS.

**The pad filter** is measured by the sampler ratchet (`tests/sampler.rs`, [fx-quality.md](fx-quality.md#the-sampler)): PF1 to PF7 hold it to the same kind of targets as F1 to F4 and F7 (the cutoff's -3 dB point within 1 %, a Butterworth's shape within 0.1 dB, 6 dB an octave a pole, the passband within 0.5 dB, swept pink at full Resonance at most -1 dBFS, a quiet sine ringing up at least 28 dB, a driven Ladder's aliases at most -65 dB), and DSP quality's Y1c and Y2b (`crate::filters`) hold its stepping under an envelope and LFO and the Ladder's singing. On main (2026-11-23) the slopes and the passband meet theirs; the cutoff, shape, resonance and the driven Ladder near the top don't yet.

**The reference for analog-style models** (`mpfree_fxlab::filter_reference`): each model the filter gap plan builds (F5) is first written as its circuit's differential equations and solved slowly and exactly: the trapezoidal rule with Newton's method run to convergence every step, at 16 times the rate (32 to check it has converged), the input brought up and the output down by the Y8 references' 120 dB filter. The real-time model, one solve a sample with its curves at twice the rate, is then measured against that render. So far it holds the linear state-variable filter (within 0.01 dB of its exact response) and the transistor ladder with its curve in every stage (Huovilainen's model, the circuit's behaviour) or on the feedback alone (the pad's Ladder today); a loud ladder's 16x and 32x renders agree to -86 dB. They differ where a ladder sings: after a click at 1 kHz, the ladder with a curve in every stage sings 20 cents flat at feedback 4.2, 70 at 5 and 100 at 6 (the circuit falls flat as it sings louder), while the feedback-only one stays within 2 cents. So a rebuilt Ladder that sounds like the circuit will sing flat at high Resonance unless its cutoff is corrected for it: a choice for F5a, against Y2b's and F9's tuning targets.

## 8. Cost

The instruction gate's cases (rustc 1.97.0, a 128-frame stereo block at 44.1 kHz): **Multi filter (LP 12)**, the clean filter at rest, 11.3k to 8.5k instructions (0.75 times: the block-at-a-time path); **Multi filter (LP 24, drive)**, 33.7k to 139.8k (4.1 times): the drive at twice the rate with ADAA, about 38k of it the ADAA's antiderivative, the rest the half-bands, the dry side's phase match and the flat FIR, as in Drive's Eco (202.6k with its tone and DC filters). The drive, filter, delay and compressor chain: 333.9k to 339.8k. **Multi filter (ladder)**, at Resonance 70 with no drive: 92.0k (the ladder at 2x and its half-bands; with drive add the ADAA's 38k). **Multi filter (LP 12, Hard)**, Resonance 70: 80.4k (the stages at twice the rate between the half-bands). With the engine and Res mode checks, (LP 12) is 8.6k and (LP 24, drive) 143.7k. The DJ filter is unchanged (13.4k).

## 9. Pitfalls and tests

| Pitfall | Guard |
|---|---|
| A resonance that clips a mix | Bounded resonance; F3; `a_loud_sound_bends_the_resonance_and_a_quiet_one_rings_up` |
| A bound that flattens quiet material | Exactly linear below the knee; F4; the same test |
| Resonance 0 or the notch changed by the bound | No bend at Resonance 0 or in the notch, bell and shelves; `with_the_drive_off_it_is_the_linear_filter_late_by_the_latency` |
| Clicks on a type or slope change | The crossfade; F5; `a_type_or_slope_change_crossfades_instead_of_clicking` |
| Fizz from the drive | 2x with ADAA; F7 |
| The sound moving in time as the drive comes and goes | A fixed latency, the dry side faded between the plain delay and the matched one |
| A block size changing the sound | The grids counted from reset and the rest path the same numbers as frame by frame; `any_block_size_sounds_the_same` and `a_fixed_delay_by_the_block_is_the_same_as_frame_by_frame` |
| Zipper on sweeps | Glides in the log domain; F6 |
| Denormals in a ringing filter fed silence | Every state flushed under 1e-20 |
| A ladder out of tune, or its oscillation pumping | The TPT solution with the cutoff pre-warped at 2x; F9; `the_ladder_sings_in_tune_on_its_own` |
| A self-oscillating ladder cut off by a chain's sleep | It never rests from Resonance 93 % up |
| A Model change clicking | The crossfade; F5's `click_multi_to_ladder`; `a_model_change_crossfades_instead_of_clicking` |
| A Res mode change clicking, or a singing ladder stopping dead | The knee moved, the engines crossfaded, the ladder's curves blended; F12; `a_res_mode_change_crossfades_instead_of_clicking` |
| Hard fizzing | The clip's rounded edge, at twice the rate; F12's alias rows (reported) |
| A Clean ladder growing without end | Its feedback stops at 3.9; F12; `the_ladder_sings_on_hard_and_rings_out_on_clean` |

## 10. History and next

- **Round 2, Filters PR (17 Nov, #482):** bounded resonance, the type and slope crossfade, the drive at twice the rate with ADAA, a fixed 4-frame latency, and the block path at rest.
- **Round 2, Ladder and Res mode PR (17 Nov):** the Ladder model (plan-2 section 7.2 item 4, F9) and the Res mode on both models (Jay's pick on decision 10.4, F12).
- **The filter gap plan (23 Nov, step F1):** the pad filter's measures, PF1 to PF7 in the sampler ratchet, the slow exact reference for analog-style models (`filter_reference`), and this page made the pad filter's design page too.
- **The filter gap plan, revised (23 Nov):** Jay's design review turned the plan into thirteen phases around the shared filter system above (F0). Step F2's first part: the new crate `mpfree-filter`, with the Filter effect's and EQ's state-variable stage moved into it bit for bit (`mpfree-fx` re-exports it; every effect test passes unchanged).
- **Next:** an LFO and envelope follower on the cutoff (an auto filter) and key tracking (decision 10.4's other questions); more ladder outputs (high-pass and band-pass from the stages, as the Oberheim Xpander mixes them) if asked for; ADAA-like antialiasing of the ladder's curve and of Hard's clip if heavy drive into them needs it. The DJ filter could take the bounded resonance if its Resonance range is ever raised.

## 11. Sources

- A. Simper, "Linear trapezoidal integrated SVF", Cytomic, 2013.
- V. Zavalishin, *The Art of VA Filter Design*, rev. 2.1, Native Instruments, 2018.
- A. Huovilainen, "Non-linear digital implementation of the Moog ladder filter", DAFx 2004.
- S. D'Angelo and V. Välimäki, "An improved virtual analog model of the Moog ladder filter", ICASSP 2013.
- T. Stilson and J. Smith, "Analyzing the Moog VCF with considerations for digital implementation", ICMC 1996.
- J. Parker, V. Zavalishin and E. Le Bivic, "Reducing the aliasing of nonlinear waveshaping using continuous-time convolution", DAFx 2016 (ADAA).
- Ableton, Live 12 audio effect reference, Auto Filter.
