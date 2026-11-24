<!--
covers: rust/mpfree-fx/src/amp.rs
checked: 2026-11-17 c87022f
-->
# Amp: design

**Status:** 2026-11-17, after FX quality round 2's Amp PR (#452). **Amp** (`mpfree.fx.amp`) is a guitar and bass amplifier with its cabinet: a gate, a boost pedal, one to three tube-like gain stages, a tone stack worked out from the circuit, a power stage that sags, presence and a speaker cabinet. Its short entry is in [fx.md](fx.md); its measured targets are in [fx-quality.md](fx-quality.md) (class `amp`, ids A1 to A12). The fuzz pedal has its own page, [fx-fuzz.md](fx-fuzz.md); single-curve drive is [fx-drive.md](fx-drive.md). The research notes and the plan are in jferments/mpfree-claude `fx/quality/` (research-amp.md, plan-2.md section 12). The Exciter, which lives in the same source file, is a different effect and not covered here.

## 1. What it is for

| Job | Typical move | What matters most |
|---|---|---|
| Clean to edge-of-breakup guitar, keys, bass | Clean or Crunch, Gain low to mid, a 1x12 or 2x12 cab | It cleans up as you play softer; no fizz; the tone knobs do what they say |
| Rock crunch and lead | British crunch, Gain 5 to 8, mids up | Harmonics that follow the notes; chords that stay readable |
| Metal and high gain | A high-gain amp, a boost in front (a Tube Screamer style pedal, drive low, level up), a gate first, a 4x12 | Tight low end on palm mutes and power chords; no aliasing on high notes; a gate that shuts hiss without chopping notes |
| Electronic and sampler work | Amp and cab on synths, drums, 808s and bass for grit | Harmonics so the low end reads on small speakers; no runaway loudness |

The MPC's Distortion Amp opens as Amp's Classic crunch preset (`mpc.rs`). Jay's guide (17 Nov): the range must reach rich, gnarly, high-gain distortion, while the defaults and presets stay broad across styles.

## 2. Principles

**An amp is a chain of filters and curves, and the order matters.** A tube preamp stage is gain, an asymmetric curve (the grid conducts on one side), a low-pass from the Miller capacitance, and a coupling capacitor (a high-pass) to the next stage. Several stages in a row each clip the last one's harmonics again, so the spectrum fills in and the sound compresses and sustains; the filters between them decide which harmonics get clipped next. The tone stack after the preamp is passive and loses 10 to 20 dB; the power stage gets it back, rounds the loudest peaks, and its supply sags under load (Aiken: for example 20 V on a 100 mA swing across 200 Ω of winding), which lowers the headroom and compresses: the "feel" a static curve lacks. The speaker cabinet then cuts steeply above about 5 kHz, which is why an amp without a cab sounds fizzy.

**The tone stack is the circuit, not an EQ.** The Fender, Marshall and Vox family's treble, mid and bass network (the "FMV" stack) is a passive third-order filter whose knobs interact: turning bass changes the mids. Yeh and Smith worked out its transfer function from the pot and capacitor values, so a digital stack can follow the real one's curves, knob for knob.

**Tight low end.** High gain on low notes makes intermodulation: difference tones under the root (a power chord of E2, B2 and E3 makes 41 Hz) that read as mud or flub. Players keep it in check by cutting bass before the gain: a model's tightening high-pass, or a Tube Screamer in front, whose clipping stage's 720 Hz high-pass and 723 Hz low-pass give its "mid hump" (Electrosmash), drive low and level up.

**Aliasing at high gain.** Real amps have none. Modellers are tested with sines above a few kHz at realistic levels, looking for tones that move down as the input moves up; about -60 dB is often quoted as where it stops mattering (forum measurements, not controlled). The more gain and the more stages, the worse it gets, so the first, hardest stage needs antialiasing most.

**Gain changes the grit, not the loudness**, so the knob can be judged by ear and comparisons are fair (the same principle as Drive's auto gain, [fx-drive.md](fx-drive.md) section 2).

**A gate first.** At high gain the amp lifts the input's hiss and hum to the level of the notes. A gate in front shuts it in rests; it must open within a few milliseconds and close with a hold and a release, so a decaying note isn't cut mid-decay and the gate doesn't chatter at its threshold.

**Mono.** An amp is mono: Amp sums its input and plays the same sound on both sides. Stereo width comes from doubling elsewhere.

## 3. Research

References studied (research-amp.md has the details and the sources): amp simulators such as Neural DSP's and Line 6 Helix (a gate first, a boost or drive in front, several amp channels, cab impulse responses with mic choices, Depth and Presence, sag); the circuit analyses of the Tube Screamer, Fuzz Face and Big Muff (Electrosmash); Aiken on sag; Premier Guitar on impulse responses (an IR captures a cab's frequency and phase response, the comb filtering between speakers and its reflections, which a few EQ bands only approximate); Wright and colleagues at Aalto on real-time amp emulation (black-box models alias about 40 dB under the first harmonic); FabFilter Saturn 2 and Ableton Roar for electronic distortion; and the MPC's own Amp.

What musicians expect, and where Amp stands:

| Feature | Common practice | Amp |
|---|---|---|
| Models | Clean, crunch, high gain, bass | Clean, Crunch, Lead, Bass (1, 2, 3 and 2 stages) |
| Tone stack | Bass, Mid, Treble from the circuit | Yeh and Smith's FMV stack, three sets of values |
| Presence, Depth | Both | Both |
| Sag | Many simulators | Sag |
| Boost in front | Standard for metal; treble boosters | Boost: Off, Tight, Treble |
| Gate first | Standard | Gate and Release |
| Cabinet | IRs, mic choices, your own IR | Four cabs of six filters each, and Off; no IRs yet |
| Stereo | Amps are mono | Mono |

**The baseline** (the Drive PR's tree, before the Amp PR): Clean and Bass aliased little, every model cleaned up as the input dropped, the stack was modelled from the circuit, and Model and Cab changes didn't spike. But Lead at Gain 10 aliased at -38, -38 and -32 dB (1 and 2 kHz at -6 dBFS, 5 kHz at -12) and Crunch at -68, -48 and -36; the trims were set on a picked note, so Lead played pink 12.5 LU over the dry at Gain 2.5; the tone knobs jumped (Treble 0 to 10 spiked 38.8 dB on Clean) and Gain zippered (up to 8.2 dB); taking the cab out clicked (38.0 dB on Clean); and there was no boost, gate, sag or depth.

## 4. Our engine

One side (the input's two sides summed), in order:

```
in ─ Gate ─ up 4x ─ Boost ─ tightening high-pass ─ stage 1 (ADAA) ─ stages 2, 3 ─ tone stack ─ Depth ─ power stage (Sag) ─ × trim ─ 12 kHz guard ─ down ─ Presence ─ Cab ─ × Level ─ out (both sides)
```

**The gate** (Gate 0 is off; 1 to 10 put the threshold at -80 to -35 dBFS) runs at the base rate on the input. It opens over half a millisecond the frame a peak passes the threshold, holds 30 ms after the level falls 6 dB under it (the hysteresis stops chatter), then fades 80 dB over Release. It sits before everything, so hiss is shut before the gain lifts it.

**4x oversampling** between polyphase IIR half-bands (Drive's Eco design, two stages of five and four all-pass sections) carries everything from the boost to the power stage, so the clipping and the filters between the stages run at 176.4 kHz. Each stage's up-sampler and down-sampler are one unrolled `oversample::IirPair`, whose states are flushed of what is too small to hear every 32 frames counted from the stream's start, as Drive's Eco does (DSP quality step 2.3B: the same output to the bit as the list-based `IirUp2` and `IirDown2` with a flush after every section, for about a tenth fewer instructions).

**Boost** (Off, Tight, Treble). **Tight** is a Tube Screamer style branch: a 720 Hz one-pole high-pass, 21.6 dB and soft clipping, added to the input, the sum cut under 120 Hz and rolled off at 3 kHz, +3 dB: the mid hump with the drive low and the level up, as metal players use it. **Treble** is a treble booster: a 1.5 kHz one-pole high-pass into 20 dB and a curve biased by 0.3, so one side clips first. Both are crossfaded on a change.

**The stages.** Each model sets a tightening high-pass, the first stage's gain at the top of Gain, the number of stages and their bias:

| Model | Tightening high-pass | First stage at Gain 10 | Stages | Bias | Stack | Depth's frequency |
|---|---|---|---|---|---|---|
| Clean | 60 Hz | 24 dB | 1 | 0.15 | Tweed | 100 Hz |
| Crunch | 100 Hz | 40 dB | 2 | 0.25 | British | 90 Hz |
| Lead | 300 Hz | 54 dB | 3 | 0.3 | Californian | 80 Hz |
| Bass | 30 Hz | 30 dB | 2 | 0.2 | Tweed | 60 Hz |

Gain sets the first stage in dB (0 to the model's top) and the later stages from 1 to 3 times. Each stage is `tanh(g x + b) - tanh(b)` (the bias makes the halves clip differently, adding even harmonics as a tube does, and subtracting `tanh(b)` keeps silence silent), then a 9 kHz Miller low-pass and a 30 Hz coupling high-pass. **Only the first stage's curve is averaged by ADAA** (log cosh, as Drive's Soft): it takes the most gain and squares the wave off. The later stages clip what its Miller pole has already smoothed; ADAA on every stage and on the power stage cost 424k and 506k instructions instead of 320k and 342k and changed the worst alias by under 0.5 dB, so they take the plain curve. A model change seeds the first stage's ADAA so its next average is over the signal's own step, not the jump in bias.

**The tone stack** is Yeh and Smith's third-order transfer function of the FMV circuit, `(b1 s + b2 s² + b3 s³) / (1 + a1 s + a2 s² + a3 s³)`, from nodal analysis with the mid pot's wiper on C3, using three sets of real pot and capacitor values: a 1959 tweed bass amp's (Yeh and Smith's reference circuit), a British lead amp's (more mid, tighter bass) and a Californian high-gain one's (bigger bass capacitors). It is made digital by the bilinear transform at 4x and run in 64-bit floats, since its poles sit close to the unit circle at low frequencies. Its values glide: they are worked out again every 4 frames while a knob moves, on a frame grid counted from the start, so any block size sounds the same. It moved inside the 4x path in the Amp PR, ahead of the power stage that moved there too; running it at the base rate again would save cost at a few dB of alias at Gain 10.

**Depth** is a bell at the model's low resonance (60 to 100 Hz), up to +10 dB, the power amp's resonance with the speaker.

**The power stage** is a gentle `tanh` whose ceiling **Sag** lowers as a supply would under load: a follower of the stage's output size (10 ms up, 150 ms down), worked out once a base frame, divides the supply by up to 1 + 1.5 times it. Loud chords compress and recover; the stack's 10 to 20 dB loss is made up before it. It clips gently and at 4x, so its plain curve aliases under the stages' (ADAA on it moved no alias row). A 12 kHz guard low-pass follows, before the down-sampler.

**Presence** is a high shelf at 3.5 kHz, 0 to 8 dB, at the base rate.

**The cabinet** (Off, 1x12 open, 2x12, 4x12, 1x15 bass) is six filter stages each (a high-pass at the low resonance, bells for the cone's bumps and the dip near 7 kHz, two low-passes for the steep fall above 4 to 6 kHz), fitted by ear and to the shape of published speaker responses. A change keeps the old cab's memory, fades it out over 10 ms and starts the new one from it (or from rest, after Off).

**Level across Gain.** Each model's output follows a trim over Gain, fitted at Gain 0 to 10 in steps of 1 (straight lines between) so pink noise at -18 dBFS RMS comes out as loud as it went in, at the default tone and cab with Boost, Sag and Depth off. The knob changes the grit, not the loudness; Level sets the rest. The old trims were set on a picked note, which is why Lead played up to 12.5 LU loud on music. (The fit was made with a scratch tool that renders 4 s of pink per point and reads BS.1770 loudness; it isn't committed. The A3 ratchet rows show any drift a later change makes.)

**Changes that don't click.** Gain, Bass, Mid, Treble and Level glide over 20 ms; a change of Model runs the old and new models side by side for 10 ms and crossfades, as Boost and Cab changes do.

Turned down: a cab from impulse responses (the step past filter cabs; loading your own IR needs the app's file picker, so it went to the app thread as a proposal); ADAA on every stage (above); cutting the Tight boost's lows harder (two poles at 150 Hz made Lead's mud worse, -15.4 dB, and its switch click).

## 5. Controls

| Control | Range (default) | Law and smoothing |
|---|---|---|
| Model | Clean, Crunch, Lead, Bass (Crunch) | 10 ms crossfade |
| Gain | 0 to 10 (5) | 20 ms glide, with its trim |
| Bass, Mid, Treble | 0 to 10 (5) | 20 ms glide; the stack every 4 frames while moving |
| Presence | 0 to 10 (3) | 0 to 8 dB at 3.5 kHz |
| Cab | Off, 1x12 open, 2x12, 4x12, 1x15 bass (4x12) | 10 ms crossfade |
| Level | -24 to +12 dB (0) | 20 ms glide |
| Boost | Off, Tight, Treble (Off) | 10 ms crossfade |
| Gate | 0 to 10 (0, off) | Threshold -80 to -35 dBFS |
| Release | 20 to 2000 ms (150) | Log |
| Sag | 0 to 10 (0) | Up to 1 + 1.5 times the load |
| Depth | 0 to 10 (0) | Up to +10 dB at the model's resonance |

Ids 0 to 7 are the first version's; Boost, Gate, Release, Sag and Depth (8 to 12) were appended by the Amp PR and default to off, so saved projects keep their settings.

## 6. Defaults and presets

**The default** is Crunch at Gain 5 with the tone knobs at noon, Presence 3 and a 4x12: a recognisable amp sound in the middle of the range, neither clean nor metal. Boost, Gate, Sag and Depth are off by default, so an insert sounds as it did before they existed.

**Presets**, broad across styles and each within 0.1 LU of the dry on pink at -18 dBFS (Level set by measurement):

| Preset | Settings | Job |
|---|---|---|
| Sparkle clean | Clean, Gain 3, Treble 7, 1x12 open | Clean chime |
| Edge of breakup | Clean, Gain 7, Mid 6, Treble 6, Sag 5, 2x12 | Cleans up with your touch; sag for feel |
| Classic crunch | Crunch, Gain 6, 4x12 | Rock rhythm (the MPC's Distortion Amp) |
| Modern lead | Lead, Gain 8, Mid 3, 4x12 | High-gain lead |
| Tight rhythm | Lead, Gain 7, Mid 4, Boost Tight, Gate 5, Depth 4, 4x12 | Tight high-gain rhythm: boost, gate and depth together |
| Treble lead | Crunch, Gain 6, Mid 6, Boost Treble, 2x12 | A treble-boosted British lead |
| Bass grit | Bass, Gain 4, Bass 6, 1x15 | Bass with grit that keeps its low end |
| Synth grit | Crunch, Gain 4, Bass 6, Treble 6, Presence 5, 1x12 open | Amp colour on synths |
| Lo-fi radio | Crunch, Gain 7, Bass 0, Treble 3, 1x12 open | A thin, broken-speaker sound |

## 7. Measuring quality

`fxlab quality --class amp` measures each model and preset ([fx-quality.md](fx-quality.md), Amp (A)). Its own measure, **power chord mud**: E2, B2 and E3 together through the amp, the share of the output from 20 to 70 Hz against all of it from 20 Hz up.

| Id | What | Target |
|---|---|---|
| A1 | Worst alias at Gain 10 of 1 and 2 kHz at -6 dBFS and 5 kHz at -12 dBFS | ≤ -60, -60 and -50 dB, every model |
| A2 | THD of 1 kHz from -6 to -30 dBFS at Gain 5 | Falls at least 10 dB (Clean, Crunch, Bass; Lead reported) |
| A3 | Pink at -18 dBFS across Gain 2.5 to 10 | Within ±2 LU of the dry |
| A4 | Presets on pink at -18 dBFS | Within ±1 LU |
| A5 | Power chord mud at Gain 10, cab off | Reported plain; ≤ -25 dB on Lead with the Tight boost |
| A6 | A tone knob thrown 0 to 10 under a 220 Hz sine | ≤ 3 dB over the steady sound |
| A7 | fxlab's zipper on Gain | ≤ 3 dB |
| A8 | Model, Cab and Boost changes | ≤ 3 dB |
| A9 | Silence; with Gate 8 at Lead 10: hiss at -60 dBFS in a rest, a note's start, a decaying note | ≤ -120 dBFS; the rest ≤ -90 dBFS; within 1 dB of ungated in ≤ 2 ms; the gate shuts once |

After the Amp PR all 76 are met (38 of 64 before). Lead at Gain 10 aliases at -64.1, -64.3 and -57.6 dB and Crunch at -83.0, -73.8 and -61.5; pink stays within 0.4 LU of the dry from Gain 0 to 10 on every model and every preset within 0.1 LU; Tight takes Lead's power-chord mud from -13.6 to -27.9 dB; a thrown tone knob spikes at most 0.05 dB, Gain's zipper is 0.08 dB, a Cab change reads 0 and a Boost change at most 0.8. Reported: on the other models the Tight boost adds mud instead (its own clipping makes the chord's difference tones, which only Lead's 300 Hz tightening high-pass then takes out, as a real overdrive into a clean amp does).

Listening: the Amp PR's blind, loudness-matched A/B page (main against the PR, six cases: Modern lead on a high melody, Classic crunch on rhythm chords, Gain from 0 to 10 on Lead, Sparkle clean with Treble thrown, Tight rhythm with hiss in the rests, Bass grit) is https://claude.ai/artifact/J8nwDAvkg8LAv1bD1TQAfm.

## 8. Cost

The instruction gate (rustc 1.97.0, a 128-frame stereo block at 44.1 kHz): Amp (Crunch) 319.6k instructions (206.6k before the Amp PR, 1.55 times), Amp (Lead, Gain 10) 341.6k (232.8k, 1.47 times). The unrolled half-bands of DSP quality step 2.3B took 10.3 % and 9.6 % off those ([DSP quality](dsp-quality.md#step-23b-part-1-the-half-bands-bookkeeping)). The gate, the boost and Depth cost nothing while off; Sag's follower runs once a frame. The device round measures the real share (amp-device-checks.md in project files). The lever if it is too dear: the tone stack back at the base rate, at a few dB of alias at Gain 10.

## 9. Pitfalls and tests

| Pitfall | Guard |
|---|---|
| Fizz at high gain | 4x with ADAA on the first stage; A1 |
| Loudness jumps with Gain or Model | Trims fitted on pink at every step of Gain; A3, A4 |
| Mud on chords | The models' tightening high-pass; the Tight boost; A5 |
| Clicks from knobs, Model, Cab, Boost | Glides, the stack every 4 frames, 10 ms crossfades; A6 to A8; `changes_and_thrown_knobs_dont_click` |
| Hiss in rests; a gate that chops notes or chatters | The gate's hold, hysteresis and release; A9; `the_gate_shuts_hiss_opens_at_once_and_shuts_once`, `silence_stays_silent_with_every_model_boost_and_the_gate` |
| A dead, static feel | Sag; `sag_compresses_a_loud_chord_and_recovers` |
| Depth doing nothing | `depth_lifts_the_lows` |
| A block size changing the sound | The stack's frame grid; `any_block_size_sounds_the_same_while_knobs_glide` |
| A stereo synth thinning out when summed | Known: Amp is mono by design |

## 10. History and next

- **Before round 2:** four models with the FMV stack at the base rate, stages at 4x without ADAA, the power stage at the base rate, trims on a picked note, filter cabs.
- **Round 2, Amp PR (#452, 17 Nov):** ADAA on the first stage, the stack and the power stage inside 4x, trims on pink, glides and crossfades, Boost, Gate and Release, Sag and Depth, four presets (Edge of breakup, Tight rhythm, Treble lead, Synth grit).
- **Next:** your own cab IR (a WAV from the browser), proposed to the app thread, since it needs the file picker; a re-fit tool for the trims committed to fxlab, so a later change to a model can re-fit them in one command; the device round's cost numbers.

## 11. Sources

- D. T. Yeh and J. O. Smith, "Discretization of the '59 Fender Bassman tone stack", DAFx 2006 (the FMV stack's transfer function).
- Electrosmash: "Tube Screamer circuit analysis", "Fuzz Face analysis", "Big Muff Pi analysis".
- Aiken Amplification, "What is 'sag'?".
- Premier Guitar, "The working guitarist: all about impulse responses".
- A. Wright and colleagues, "Real-time guitar amplifier emulation with deep learning", Aalto University.
- The Gear Forum, "Modelers and aliasing" (forum measurements and methods).
- Waves, "3 bold and creative ways to use distortion in your productions".
