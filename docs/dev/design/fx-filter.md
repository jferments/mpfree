<!--
covers: rust/mpfree-fx/src/svf.rs
checked: 2026-11-17 6a38168
-->
# Filters: design

**Status:** 2026-11-17, after FX quality round 2's Filters PR. The **Multi filter** (`mpfree.fx.filter`) and the **DJ filter** (`mpfree.fx.dj_filter`) are the Filter family: one effect in the picker with two modes. Their short entries are in [fx.md](fx.md); their measured targets are in [fx-quality.md](fx-quality.md) (ids F1 to F11). The research notes and the plan this page draws on are in jferments/mpfree-claude `fx/quality/` (research-filter.md, plan-2.md section 7). The EQ is built on the same state-variable filter (its design is in fx.md's EQ entry); the Auto wah, the Comb and the XY FX filter are separate engines.

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

**Resonance in analog filters saturates.** A linear filter's peak grows with Q without limit: at Q 40 a loud sound at the cutoff comes out 32 dB over itself, and a mix-level loop swept through it clips. In an analog filter the resonance loop runs into the limits of its transistors or op-amps, so the peak compresses as the signal grows and the filter rings up fully only on quiet material (the Moog ladder's tanh stages, Huovilainen DAFx 2004). Bounding the loop's state does the same in a digital filter.

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

**Bounded resonance** (decision 10.4: simply how the filter works, not a choice). In the resonant stages (a low-pass's or high-pass's only stage, the second at 24 dB, and both of a band-pass's) the band-pass state is bent as it is worked out: unchanged up to a knee, then `knee + room · u / (1 + u)` with `u` how far past the knee it is over `room`, so it approaches `knee + room` smoothly and never passes it. The low-pass state integrates the bent band-pass (`v2 = ic2 + g · v1`), so the whole loop is bounded, not just the output. At Resonance 25 % and up the knee is 0.35 and the room 0.3 (full scale 1); below 25 % both grow as 1 over (Resonance / 25 %), so a gentle filter never bends a mix-level sound; at Resonance 0 nothing bends and the stages are the plain linear SVF, bit for bit. The notch, bell and shelves don't ring up at their output and stay linear. The result: a -40 dBFS sine at the cutoff still rings up 31 dB at full Resonance (32 dB linear), while pink at -18 dBFS RMS through a swept low-pass at full Resonance peaks at -2.0 dBFS (12 dB) and -3.1 (24 dB), from +2.8 and +1.0.

Chosen while building: bending the band-pass state rather than the input or the output (bending the input leaves the peak unbounded; bending the output flattens the passband of a loud sound and leaves the loop's state to grow); a rational knee rather than `tanh` (no transcendental per sample, and exactly linear below the knee, so quiet material and Resonance 0 are untouched); the knee scaled down below 25 % Resonance rather than one knee for all (a fixed knee bent loud drums through a flat low-pass).

**Type and slope changes** crossfade over 10 ms with a smoothstep. The old filter keeps running from its state while the new one takes over; the new one starts from the old one's state (the 12 dB filter's types share it), and a second stage newly in use starts from silence. A change in the first half of a fade fades from the same old filter; later, from the one that was coming in, so a fast run of changes never jumps.

**Drive** (0 to 24 dB) runs at twice the rate: Drive's Eco half-bands (polyphase IIR, five all-pass sections, flat to 0.19 of the doubled rate and 71 dB down from 0.31), the soft curve `x(27 + x²)/(27 + 9x²)` averaged by first-order ADAA (its antiderivative worked out in 64 bits), then the short flat FIR that makes ADAA's two-point mean flat to 20 kHz. It blends in over the first 3 dB, against a dry side taken through the half-bands' phase and the same delay, so the two add in phase. With the drive off the sound takes a plain 4-frame delay instead, and when the drive comes in or leaves the dry side moves between the plain delay and the matched one over 256 frames. So the **latency is a fixed 4 frames** (reported for compensation; it was 0), the same with the drive on or off, and the drive can be automated without the sound moving in time. The half-bands' states are flushed of values too small to hear every 64 frames on a grid counted from reset.

**At rest** (no glide, no fade, the drive off: most of the time) the delay and each stage run a block at a time with their state in registers, the same numbers as frame by frame; the clean filter costs less than before this PR (see section 8).

**Glides.** Cutoff (in the log domain), Resonance, Gain and Drive glide over about 10 ms, the coefficients worked out again each frame while they move.

**The drawing** (`MultiFilter::response_db`) is the linear filter's response, exact for quiet material; a loud sound at a high Resonance rings less than drawn, by design.

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
| DJ: Position | -100 to +100 (0) | Low-pass left, high-pass right, off in the middle; 15 ms glide |
| DJ: Resonance | 0 to 100 % | Q 0.707 to 6.7; 15 ms glide |

## 6. Defaults and presets

**The default** is an open 12 dB low-pass at 20 kHz with no resonance or drive: it passes the sound, 4 frames late, so adding it changes nothing until the cutoff moves.

**Presets** (`presets.rs`), unchanged by this PR: Warm low-pass (24 dB at 2.5 kHz, Resonance 20), Acid squelch (24 dB at 900 Hz, Resonance 80, Drive 9 dB), Thin high-pass (300 Hz, Resonance 10), Radio band (band-pass at 1.5 kHz, Resonance 30, Drive 6 dB), Hollow notch (24 dB at 1 kHz) and Sub lift (a low shelf). Acid squelch and Radio band now drive at twice the rate, and Acid squelch's resonance bends on loud notes instead of clipping.

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
| F9 | Self-oscillation in tune | Only for a ladder model, not built |
| F10 | Zipper on every preset | Reported |
| F11 | Cost | The clean filter ≤ 1.2 times before; the drive states its own |

All 30 are met: swept pink peaks at -2.0 and -3.1 dBFS (were +2.8 and +1.0); a quiet sine rings up 31.1 dB; a type change reads 0 dB and a slope change 1.2 (were 63.5 and 29.3); the drive's alias is -71.3 dB at Drive 12 (was -19.7; -41.1 at Drive 24, reported); the cutoffs, passband, sweeps and the DJ filter are as they were. `tests/filter.rs` checks that with the drive off the filter is the linear one exactly, 4 frames late, at any Resonance 0 setting and for the notch and bell; that a loud sound bends and a quiet one rings up; that a type or slope change makes no step bigger than the tone's own; and that any block size sounds the same through a change of type, slope and drive.

Listening: a blind, loudness-matched page (main against this PR: Acid squelch swept on a bass line, a full-Resonance sweep over a loop, low-pass to high-pass and 12 to 24 dB on drums, drive into a low-pass on a lead, and Warm low-pass on keys, which should sound the same) is https://claude.ai/artifact/Wa7nNrn45aHqJ1iw12TSVS.

## 8. Cost

The instruction gate's cases (rustc 1.97.0, a 128-frame stereo block at 44.1 kHz): **Multi filter (LP 12)**, the clean filter at rest, 11.3k to 8.5k instructions (0.75 times: the block-at-a-time path); **Multi filter (LP 24, drive)**, 33.7k to 139.8k (4.1 times): the drive at twice the rate with ADAA, about 38k of it the ADAA's antiderivative, the rest the half-bands, the dry side's phase match and the flat FIR, as in Drive's Eco (202.6k with its tone and DC filters). The drive, filter, delay and compressor chain: 333.9k to 339.8k. The DJ filter is unchanged (13.4k).

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

## 10. History and next

- **Round 2, Filters PR (17 Nov):** bounded resonance, the type and slope crossfade, the drive at twice the rate with ADAA, a fixed 4-frame latency, the block path at rest.
- **Next:** a Moog-style ladder as a Model choice (plan-2 section 7.2 item 4, F9), if Jay picks it; an LFO and envelope follower on the cutoff (an auto filter) and key tracking are the other candidates. The DJ filter could take the bounded resonance if its Resonance range is ever raised.

## 11. Sources

- A. Simper, "Linear trapezoidal integrated SVF", Cytomic, 2013.
- V. Zavalishin, *The Art of VA Filter Design*, rev. 2.1, Native Instruments, 2018.
- A. Huovilainen, "Non-linear digital implementation of the Moog ladder filter", DAFx 2004.
- T. Stilson and J. Smith, "Analyzing the Moog VCF with considerations for digital implementation", ICMC 1996.
- J. Parker, V. Zavalishin and E. Le Bivic, "Reducing the aliasing of nonlinear waveshaping using continuous-time convolution", DAFx 2016 (ADAA).
- Ableton, Live 12 audio effect reference, Auto Filter.
