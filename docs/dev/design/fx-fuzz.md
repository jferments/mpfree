<!--
covers: rust/mpfree-fx/src/fuzz.rs
checked: 2026-11-17 c87022f
-->
# Fuzz: design

**Status:** 2026-11-17, new in FX quality round 2 (the Fuzz PR, #453; decision 10.6). **Fuzz** (`mpfree.fx.fuzz`) is a fuzz pedal with three circuits, Face, Muff and Octave, and the controls fuzz players reach for: Fuzz, Tone, a starving Gate, Low keep for a clean sub, and Level. Its short entry is in [fx.md](fx.md); its measured targets are in [fx-quality.md](fx-quality.md) (class `amp`, ids A3, A4, A8 to A11 and A13). Single-curve drive, Drive's own Fuzz mode included, is on [fx-drive.md](fx-drive.md); the guitar amp is on [fx-amp.md](fx-amp.md). The research notes and the plan are in jferments/mpfree-claude `fx/quality/` (research-amp.md, plan-2.md sections 10.6 and 12).

## 1. What it is for

Fuzz is the most extreme of the classic pedal distortions: a square-ish wave with long sustain, made by a few transistors driven far past clipping. People use it on guitar and bass, and in electronic music on synth leads, 808s and drums. What players look for:

- **Sustain**: a held note stays loud as it decays, until it suddenly doesn't.
- **Sputter** when the circuit is starved ("gated" or "velcro" fuzz): a dying note breaks up and cuts out, a wanted sound, not a fault.
- **Clean-up**: playing softer, or turning the source down, cleans the sound up.
- **Octave fuzz**: an octave up on single notes high on the neck, ring-modulator clusters on chords.
- **Thick, scooped fuzz** (the Big Muff family): two clipping stages with filters between and a tone control that dips the mids.
- **A solid sub under the grit**, for 808s and bass: the harmonics carry the low end on small speakers while the fundamental keeps its weight.

Why an effect of its own (decision 10.6, Jay 17 Nov 03:35Z): Drive is one curve with tone around it. The rich fuzzes are cascaded stages with filters between, a starved bias, or a rectifier into a fuzz, which are circuits, not curves. The alternative was adding Muff and Gate as Drive modes, which would have put controls on Drive's page that only some modes use. Drive keeps its curves, its Fuzz mode included. Jay's guide (17 Nov 03:29Z): make rich, gnarly distortion possible without aiming the effects at one style; defaults and presets stay broad.

## 2. Principles

**The circuits** (Electrosmash's analyses, and the research's reading of them):

- **Fuzz Face**: two transistors, about 19.5 dB at the second collector, with an asymmetric bias that clips the negative half first on small signals and both halves, squarer, as the fuzz rises. Its famous clean-up with the guitar's volume knob comes from its level and the pickup's interaction with its very low input impedance (about 5 to 8 kΩ). One curve, biased, is the core of it.
- **Big Muff**: four cascaded stages, two of them clipping through diode pairs in their feedback (about 23 and 25 dB each), band-limited around 90 Hz to 1.2 kHz inside the clipping stages, then a passive tone control whose middle notches about 1 kHz by 6.5 dB. Each stage clips the last one's harmonics again, so the spectrum fills in: thick, long sustain.
- **Octave fuzz** (Octavia, Superfuzz): a rectifier doubles the frequency before the fuzz. Strongest on single notes; on chords, the sums and differences make clusters.
- **Gated fuzz** (Z.Vex Fuzz Factory): a Gate control rebiases the first transistor; starved of bias, it needs more signal before it conducts, so quiet signals don't pass at all and decays sputter.

**Sustain is compression.** A curve driven 30 to 40 dB past its knee puts out nearly the same level whatever comes in, so a note 20 dB quieter comes out only a dB or two quieter. **Sputter is a dead zone.** A starved first stage passes nothing under a threshold; as a note decays through it, the wave's peaks poke through and its middle is cut, then nothing passes.

**Aliasing.** A square-ish wave has harmonics to every order, so fuzz aliases more than anything else in the family. Fuzz uses the same remedy as Drive and the Amp: oversampling with antiderivative antialiasing on the first, hardest curve ([fx-drive.md](fx-drive.md) section 2 explains ADAA). A dead zone has corners, so its antiderivative must be piecewise for ADAA to average them.

**Bass-safe distortion.** Hard distortion of a sub's fundamental moves its energy into harmonics; the sub thins. Keeping the band under a crossover dry, and the fuzz above it, keeps the weight (Saturn and Roar do it with bands; a crossover around 100 to 150 Hz is the research's judgement). The catch: a bare 808 has almost nothing above 150 Hz, so fuzzing only what is above the crossover leaves it clean.

**Fuzz changes the grit, not the loudness**, as Drive's auto gain and the Amp's trims do, so the knob can be judged by ear.

## 3. Research

References studied (research-amp.md): Electrosmash's Fuzz Face, Big Muff and Tube Screamer analyses; David Morrin's notes on octave-up fuzz topologies; MusicRadar on the Z.Vex Fuzz Factory (Gate, Comp and Stab); Waves on distortion in production (harmonics carry an 808 on small speakers; parallel and bass-safe distortion keep the weight); FabFilter Saturn 2 and Ableton Roar (multiband distortion). The MPC has a Distortion Fuzz and AIR Fuzz-Wah, which open as Drive's Fuzz box preset today (`mpc.rs`); neither has been measured (FXProbe hasn't run).

Before this effect, MPFree's fuzz was Drive's Fuzz mode: one exponential curve with Bias, after the Drive PR antialiased in 2x and 4x. It sustains (THD -6.7 dB at -6 dBFS, -11.8 at -30) but has no cascaded stages, no gate and no filtering between stages; Rectify is the octave without the fuzz after it.

## 4. Our engine

Stereo: each side runs on its own, at 4x between the Amp's polyphase IIR half-bands (two stages, of five and four all-pass sections, each stage one unrolled `IirPair` flushed every 32 frames from the stream's start), with a 12 kHz guard low-pass before the down-sampler:

```
in ─ up 4x ─ model (Face, Muff or Octave) × trim ─ 12 kHz guard ─ down ─ high-pass at the crossover ─┐
 └──── Low keep: the dry's low band from the same crossover ──────────────────────────────────────── + ─ × Level ─ out
```

**The first curve** of every model is the starved, biased `tanh`: `tanh(dz(u) + b) - tanh(b)`, where `dz` takes Gate's dead zone off the size of `u` (nothing passes under it), averaged by first-order ADAA. Its antiderivative is piecewise: `tanh(b) · u` inside the zone, and `±tanh(b) · d + log cosh(u ∓ d + b) - log cosh(b)` outside it, so the average follows the zone's corners without folding them. When Gate moves, the ADAA state is reseeded from the last input, so the next average is over the new curve.

| Model | What runs at 4x | Fuzz's range | Why |
|---|---|---|---|
| Face | A 30 Hz coupling high-pass, the first curve with bias -0.35, then Tone: a one-pole low-pass from 1.5 to 12 kHz (log) | 6 to 42 dB | One biased curve: one half clips first on small signals (even harmonics), both square off as Fuzz rises, and it cleans up as the input drops |
| Muff | Two stages, each a 100 Hz high-pass, gain, a symmetric clip and a 1.2 kHz low-pass (the first stage is the starved curve with no bias; the second `tanh` at 6 to 24 dB), then the scooped tone: a 400 Hz low-pass and a 1.6 kHz high-pass blended by Tone | 6 to 30 dB (first stage) | Two clipping stages with the Big Muff's in-stage band limits; the blend of a low-pass and a high-pass dips the mids at noon |
| Octave | A rectifier `|u| + u/4` (mostly the octave, a little of the note; averaged by ADAA too), its offset taken out at 30 Hz, then the first curve with bias -0.2 and Face's Tone | 6 to 44 dB | A rectifier into the fuzz: the octave up, ring-mod clusters on chords |

The Octave's top gain is 44 dB, not the 36 first drafted: at 36 its output fell 9 dB for a 20 dB drop in (the rectified octave sits about 7.5 dB under the note, so the curve needs more gain to sustain), and at 48 its 5 kHz alias was -50.5 dB, at the target's edge.

**Gate** starves the first stage: a dead zone of up to 0.05 of the input (-26 dBFS at Gate 100), growing with the square of the knob, so the low half of the knob is a gentle tightening and the top half the sputter. In Face and Muff, Gate 70 makes a decaying note break up at -31 dBFS; without it the note holds on to -55 dBFS or below.

**Low keep** (Off, 60, 100, 150, 250 Hz). A fourth-order Linkwitz-Riley crossover: the fuzz's output is high-passed at the chosen frequency, and the dry signal's low band from the same crossover is added back. The fuzz still hears the whole input, so a bare 808 still drives it, and the sub that comes out is the dry sub (a 50 Hz sine within 0.13 dB at 150 Hz and full fuzz). Linkwitz-Riley because its two halves sum flat in magnitude with matched phase, so where the fuzz is quiet the crossover adds nothing. Turned down: fuzzing only what is above the crossover (left a bare 808 clean).

**Level across Fuzz.** Each model's output follows a trim over Fuzz fitted at 0 to 100 in steps of 10 (straight lines between) so pink noise at -18 dBFS RMS comes out as loud as it went in, at Tone 50, Gate 0 and Low keep off; Level (-24 to +12 dB) sets the rest. (The fit was made with a scratch tool rendering 4 s of pink per point and reading BS.1770 loudness; it isn't committed, and the A3 rows on the ratchet show any drift a change makes.)

**Changes that don't click.** Fuzz, Tone, Gate and Level glide over 20 ms, the gains and curve settings worked out again each frame while they move. A Model change runs both models for 10 ms and crossfades. Low keep fades in and out over 5 ms and glides between frequencies over 20 ms.

Turned down: a 2x path like Drive's Eco (the device round decides whether cost needs it); ADAA on the Muff's second stage (it clips what the first stage's 1.2 kHz low-pass has already smoothed, as the Amp's later stages do); modelling the Fuzz Face's interaction with a guitar's pickup (MPFree's sources are samples and synths, not pickups; the clean-up comes from the level instead).

## 5. Controls

| Control | Range (default) | Law and smoothing |
|---|---|---|
| Model | Face, Muff, Octave (Face) | 10 ms crossfade |
| Fuzz | 0 to 100 % (60) | Linear in dB over the model's range; 20 ms glide |
| Tone | 0 to 100 % (50) | Face, Octave: a low-pass from 1.5 to 12 kHz, log; Muff: the low-pass and high-pass blend; 20 ms glide |
| Gate | 0 to 100 % (0) | A dead zone up to 0.05 of the input, with the square of the knob; 20 ms glide |
| Low keep | Off, 60, 100, 150, 250 Hz (Off) | 5 ms fade in and out, 20 ms glide between frequencies |
| Level | -24 to +12 dB (0) | 20 ms glide |

The page: Model, Fuzz and Tone; a Gate section; Low keep and Level.

## 6. Defaults and presets

**The default**, Face at Fuzz 60 and Tone 50, is a recognisable vintage fuzz at the middle of every range; Gate and Low keep start off, so it is a plain pedal until asked.

**Presets**, each within 0.05 LU of the dry on pink at -18 dBFS (Level set by measurement), spread across the jobs in section 1 rather than one style:

| Preset | Settings | Job |
|---|---|---|
| Vintage face | Face, Fuzz 70, Tone 55 | Classic fuzz on rhythm and leads |
| Thick muff | Muff, Fuzz 75, Tone 45 | Thick, scooped sustain |
| Octave lead | Octave, Fuzz 70, Tone 65 | Octave-up leads |
| Sputter | Face, Fuzz 85, Gate 60 | Gated, sputtering decays |
| 808 fuzz | Muff, Fuzz 60, Tone 60, Low keep 150 Hz | Grit on an 808 with the sub kept |
| Synth grit | Face, Fuzz 35, Tone 70 | Light grit on synths |

## 7. Measuring quality

`fxlab quality --class amp` measures the Fuzz at each model (`fuzz/face`, `fuzz/muff`, `fuzz/octave`) and each preset ([fx-quality.md](fx-quality.md), the Fuzz table under Amp (A)):

| Id | What | Target |
|---|---|---|
| A3 | Pink at -18 dBFS at Fuzz 0, 25, 50, 75 and 100 | Within ±2 LU of the dry |
| A4 | Presets | Within ±1 LU |
| A8 | Model and Low keep changes | ≤ 3 dB over the sound |
| A9 | Silence at full fuzz, Gate 0 and 100 | ≤ -120 dBFS |
| A10 | Sustain: a 220 Hz sine 20 dB quieter at full fuzz; break-up: a note decaying 40 dB a second from -12 dBFS with Gate 70 | ≤ 6 dB quieter; breaks up while still above -40 dBFS |
| A11 | Worst alias of a 5 kHz sine at -12 dBFS at full fuzz | ≤ -50 dB, every model |
| A13 | A 50 Hz sine at -12 dBFS, Low keep 150 Hz, full fuzz | Within ±1 dB of how it went in |

All 45 are met: aliases -65.9 (Face), -72.5 (Muff) and -54.0 dB (Octave); a 20 dB drop in drops the output 1.0, 0.1 and 2.5 dB; with Gate 70 notes break up at -31.2, -31.2 and -27.2 dBFS (without it -57.6, -70 and -55.2); pink within 0.1 LU at every Fuzz and presets within 0.05 LU; a Model change at most 0.5 dB, Low keep on at most 1.6; the sub within 0.13 dB. Sixty ratchet rows hold them.

Listening: a blind, loudness-matched page (the new Fuzz against what MPFree had for each job: Drive's Fuzz box preset, or its Rectify mode for the octave; five cases on plucked-string guitar and a sine 808 made in MPFree: Vintage face on rhythm chords, Thick muff on a lead line, 808 fuzz on an 808 line, Sputter on a lead line, Octave lead against Drive's Rectify) is https://claude.ai/artifact/CXxF9FY5cb5RrRzNg3ELip.

## 8. Cost

The instruction gate's `Fuzz` case (the default Face at Fuzz 60; rustc 1.97.0, a 128-frame stereo block at 44.1 kHz): 386.5k instructions. It runs in stereo (the Amp is mono), each side at 4x with ADAA on its first curve, so it costs a little less than Drive HQ (430.3k) and more than the Amp (320k to 342k). The unrolled half-bands of DSP quality step 2.3B took 17 % off it (386.5k to 320.7k on the same machine; [DSP quality](dsp-quality.md#step-23b-part-1-the-half-bands-bookkeeping)): before, the half-bands were a third of the effect. The device round measures its real share (fuzz-device-checks.md in project files); if it is over about 4 % of a period, the lever is a 2x path as Drive's Eco, at some alias on the Octave.

## 9. Pitfalls and tests

| Pitfall | Guard |
|---|---|
| Fizz | 4x and ADAA on the first curve and the rectifier; A11 |
| Loudness that follows Fuzz | Trims fitted on pink; A3, A4 |
| A gate that hisses or never shuts | The dead zone passes nothing under it; A9; `silence_stays_silent_with_every_model_the_gate_and_low_keep` |
| No sustain, or no sputter | A10; `it_sustains_and_with_the_gate_up_breaks_up_as_a_note_dies` |
| A thin sub on 808s | Low keep; A13; `low_keep_leaves_the_sub_as_it_came_in` |
| Clicks from Model, Low keep or thrown knobs | Crossfades, fades and glides; A8; `changes_and_thrown_knobs_dont_click` |
| A block size changing the sound | `any_block_size_sounds_the_same_while_knobs_glide` |
| A mono input coming out uneven | Both sides run the same code from the same state; `tests/fuzz.rs` checks a mono input comes out the same on both sides |

## 10. History and next

- **Round 2, Fuzz PR (#453, 17 Nov):** the new effect, with its rows in the amp class.
- **Next:** the MPC's Distortion Fuzz and AIR Fuzz-Wah still open as Drive's Fuzz box (`mpc.rs`); pointing them at this effect is a small change for the MPC import's owner to decide. A committed re-fit tool for the trims (shared with the Amp's). After the device round, a 2x path if the cost asks for one. A **Comp** control (the Fuzz Factory's bias on the last stage) and an oscillating **Stab** are candidates if Jay wants more of the gated family.

## 11. Sources

- Electrosmash: "Fuzz Face analysis", "Big Muff Pi analysis", "Tube Screamer circuit analysis".
- D. Morrin, octave-up fuzz topologies.
- MusicRadar, "The FX files: Z.Vex Fuzz Factory".
- Waves, "3 bold and creative ways to use distortion in your productions".
- S. Linkwitz, "Active crossover networks for noncoincident drivers", JAES 24(1), 1976 (the Linkwitz-Riley crossover).
- J. Parker, V. Zavalishin and E. Le Bivic, "Reducing the aliasing of nonlinear waveshaping using continuous-time convolution", DAFx 2016 (ADAA).
