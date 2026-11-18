# Bass and the 808: what makes a great one

**Status:** research, 2026-11-18 (synth library stage 1, wave 1). Feeds the bass engine planned in the synth library plan ("Synth library: stock-take and research plan", https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). Acid bass has its own page, [Acid bass](acid-bass.md), and the [Squelch engine](../design/synths.md#squelch-mpfree-synth-squelch) is built from it; this page covers everything else a bass engine plays: the classic mono synth, the 808 bass, the reese and the plain sub. Nothing here has run on the MPC.

Every synth research page in this stage answers the same eight questions, so the pages compare side by side. Where a point comes from a source it is linked; where it is our own reading, arithmetic or a target we propose, it says so.

## 1. What makes a great one

- **Weight that survives small speakers.** Most listeners hear bass on phones and laptops that can't reproduce 40 to 60 Hz. What makes the note heard there is its upper harmonics: "the higher frequencies are audible on small speakers and trick the brain into thinking it can hear the sub" ([MusicTech](https://www.musictech.com/tutorials/subs-for-small-speakers/)). A great bass carries its pitch in the second to fifth harmonics without losing the sub underneath.
- **In tune, all the way down.** An 808 bass "must be in key" ([Violet Recording](https://violetrecording.com/how-to-design-an-808/)), and an out-of-tune sub against a mid-bass layer causes phase trouble (MusicTech). Pitch errors are easy to hear on a long, pure low note.
- **A tight, punchy start with no click.** The 808 bass drum's circuit briefly raises its resonance "by more than an octave during the attack" for about 6 ms, too short to hear as pitch but making the attack punchier ([Werner, Abel and Smith, DAFx 2014](https://dafx14.fau.de/papers/dafx14_kurt_james_werner_a_physically_informed,_ci.pdf)). Akai's Bassline offers a **Start Phase** of Free, 0° or 180° (MPC 3.6 guide, p. 400), which is how a sine body starts without a click or with a deliberate thump.
- **Glide that feels played.** In trap, overlapping notes slide into each other ("a small amount of glide", longer for obvious slides; Violet Recording). On mono synths, whether a held note retriggers the envelopes, and which held note wins, shapes how lines feel under the fingers ([Sound On Sound, "Duophony"](https://www.soundonsound.com/techniques/duophony)).
- **A filter that growls under drive.** The transistor ladder's warmth comes from the input level driving its nonlinear stages, not from extra coefficients ([Huovilainen, DAFx 2004](https://dafx.de/paper-archive/2004/P_061.PDF)).
- **Mono, steady low end.** Keep low frequencies mono so the bass stays centred (Violet Recording). A reese's detuned oscillators make its level rise and fall ("a rather inconsistent volume level", [Attack Magazine](https://www.attackmagazine.com/technique/tutorials/reese-bass-redux/)), which is its character above the sub but a fault inside it (our reading).

## 2. How musicians use it

| Job | What it is | Typical settings (sources above; our reading where unsourced) |
|---|---|---|
| 808 bass | A sine body, pitched, often long, played as the bass line under trap and modern hip-hop | Sine in the second octave; a fast pitch drop of a few semitones or more; fast attack, full sustain, release sets the tail; distortion for small speakers; glide between overlapping notes; sidechained or ducked against the kick |
| Mono synth bass | Saw or pulse through a resonant low-pass, with a sub an octave or two down | Filter envelope for pluck or wow; key tracking low; glide on legato notes; often a 1-octave square sub |
| Reese | Two detuned saws (or sines) beating against each other, drum and bass, dubstep, darker house | Detune about ±0.27 semitone between two oscillators in Attack's starting example; mono; the beating sets the movement's tempo |
| Sub | A sine or triangle under another bass or a kick | Mono, no filter, short attack; tuned to the key |
| Acid | See [Acid bass](acid-bass.md) | Squelch |

## 3. Algorithms

- **The 808 bass drum as a circuit.** Werner, Abel and Smith model the TR-808 bass drum as functional blocks: a trigger pulse (1 ms; 4 to 14 V with accent), a pulse shaper (a low shelf and a diode clipper), a bridged-T band-pass in an op-amp's feedback ringing as a decaying pseudo-sinusoid (about 49.5 Hz in their model; Roland's chart says 56 Hz typical), a feedback buffer whose shelf sets the decay, and tone, level and output stages. Two pitch effects: the **attack shift** (over an octave up for about 6 ms) and a **pitch sigh**, a slow wander of roughly 48 to 58 Hz over the first 300 ms. Linear blocks are bilinear-transformed transfer functions in transposed direct form II with a unit delay breaking the delay-free loop; the paper gives no CPU cost ([DAFx 2014](https://dafx14.fau.de/papers/dafx14_kurt_james_werner_a_physically_informed,_ci.pdf)).
- **The 808 bass as producers build it.** A sine with a pitch envelope (up a few semitones, very fast attack, very short decay; too deep or slow sounds like a "pew"), an amp envelope with full sustain, and distortion, sometimes in parallel with a clean sub (Violet Recording). MusicTech's tip for cleaner grit: distort, then low-pass at about 300 Hz, keeping the second and third harmonics.
- **Our reading:** the circuit model is the right body for an 808 kick and a short 808 bass; for a chromatic, sustained 808 bass the pitch must settle exactly on the note, so the pitch sigh needs to be an option, not a given. Both fit one engine: a sine body with a pitch envelope (attack shift and drop), a decay or sustain, and drive.
- **The transistor ladder.** Huovilainen models each of the four stages and the input with tanh (five tanh evaluations a sample with reuse), solves each first-order stage by forward Euler, runs at twice the rate, and adds a half-sample delay in the feedback to keep the resonance on the cutoff (tuning error under 10 % below a quarter of the rate) ([DAFx 2004](https://dafx.de/paper-archive/2004/P_061.PDF)). Later work compares implementations against SPICE: nonlinearity only at the input and feedback matches the linear response but misses the harmonics, and the early stages matter most ([Oyama, DAFx 2026](https://www.dafx.de/paper-archive/2026/papers/DAFx26_paper_29.pdf)). Poly already has a zero-delay-feedback ladder; Squelch and Analog use the "tanh(d)/d from the previous sample" trick ([Acid bass](acid-bass.md)) to keep every stage nonlinear in a zero-delay solve.
- **Oscillators and sub.** The SH-101's source mixer has saw, pulse and noise levels and a sub with three modes: one octave down, two octaves down, and two octaves down with a narrow pulse ([SH-101 owner's manual, source mixer](https://www.manualslib.com/manual/2211414/Roland-Sh-101.html?page=6)). Bassline adds a **Fifth** oscillator next to its sub-octave (MPC 3.6 guide, p. 400). A sub that divides the main oscillator, rather than running free, stays in phase with it (our reading; it is how divider subs work).
- **Reese.** Two oscillators detuned against each other in mono (Attack: ±0.27 semitone on sines as a start; the original was made on a phase distortion synth). Our reading: keep a separate clean sub under it, or high-pass the detuned pair, so the beating doesn't empty the low end.

## 4. What to measure

Targets are proposals for stage 4; the numbers in brackets are ours until the baseline and listening confirm them.

| Id | Measure | Proposed target | Why |
|---|---|---|---|
| B1 | Settled pitch of a held note, 100 ms after the start, C0 to C3 | Within [±3] cents | A long, pure low note shows pitch errors |
| B2 | Glide: pitch over time between two notes | Lands within [2] cents; time matches the setting within [10] %; constant-time or constant-rate as chosen | Glide feel; tuning at the landing |
| B3 | 808 pitch envelope | Drop amount and time as set, within [5] %; settles per B1 | The punch, then in key |
| B4 | Start of a note | No step larger than [−40] dB of full scale at the first sample with Start Phase 0°; a fresh start at 0° or 180° every note | No click unless asked for |
| B5 | Small-speaker audibility | Energy between 100 and 400 Hz at least [−12] dB relative to below 100 Hz at the default drive, for C1 to C2 | Heard on phones (MusicTech) |
| B6 | Level across the keyboard | Loudness spread under [3] dB from C0 to C3 at a fixed velocity | Every note of the line as loud |
| B7 | Low end stays mono | Correlation above [0.95] below 120 Hz, any preset | Centred bass (Violet Recording) |
| B8 | DC after drive | Under [−60] dBFS | Asymmetric drive makes DC |
| B9 | Ladder response | Within [1] dB of the analog model at rest; self-oscillation within [25] cents of the cutoff below a quarter of the rate | Huovilainen's tuning; honest screen |
| B10 | Aliasing under drive | Worst alias under [−60] dB on C5 at full drive | Drive at the top of the range |
| B11 | Cost | One voice with oversampled drive and ladder under [5] % of a period in the worst block | One bass track among many |

## 5. Knobs and defaults

What the classic monos, Bassline (MPC 3.6 guide, pp. 400 to 402) and producers' 808 recipes have in common. Defaults are our proposals.

| Section | Controls | Proposed default |
|---|---|---|
| Source | Wave (saw, pulse with width, triangle, sine), Octave (32', 16', 8'), Sub level and mode (-1 oct, -2 oct, -2 oct narrow), Fifth, Noise, Start phase (free, 0°, 180°) | Saw at 16', sub -1 oct at 50 %, start phase 0° |
| 808 body | Pitch drop (semitones), Drop time, Punch (the attack shift), Decay or sustain | Off on the synth presets; on for the 808 presets (drop +12 semitones over about 30 ms, our reading) |
| Filter | Cutoff, Resonance, Env amount (±), Key track, Drive into the ladder, High-pass (Bassline has 10 to 500 Hz) | Cutoff about 800 Hz, Resonance low, Env amount +40 %, Key track 0 |
| Envelopes | Filter decay; amp attack, decay, sustain, release | Short decay, full sustain |
| Play | Glide time, Glide on legato or always, constant time or rate, Note priority (last, low, high), Envelope retrigger on or off, Bend range | Glide 60 ms on legato, last note, retrigger off |
| Output | Drive (soft, hard) with a low-pass after it, Level | Soft, low |

## 6. The presets players expect

A first canon of 32, by job (our reading of the sources and of what bass libraries usually hold): clean 808, long 808, distorted 808, 808 with slide, 808 kick-bass; sub sine, sub triangle; round mono, pluck bass, resonant sweep bass, square bass with -2 oct sub, rubber bass (slow filter decay), funk bass (fast envelope, high resonance), synth-pop bass (saw plus sub, short); reese, reese with sub, dark reese, wide reese (stereo above the sub); FM bass (from FM), acid (from Squelch); and a few that use pressure (growl on press) and the Fifth (power bass).

## 7. Playing it on the MPC

- **Pads as keys, mono.** Pads overlap easily, so legato-on-overlap with glide makes slides playable by holding one pad and tapping the next (our reading).
- **Pressure.** Into drive or cutoff for growl; vibrato on long 808s.
- **Q-Links.** Cutoff, Env amount, Drive, Glide for the synth basses; Drop, Decay, Drive, Tone for the 808s.
- **The 808's tail.** Long tails overlap the next kick: ducking against the kick is a common move (Violet Recording). The engine can't duck by itself; the mixer's ducker does, so presets should say how they're meant to sit.

## 8. What goes wrong

- Pitch that wanders on a long 808 (a model's pitch sigh left on), or an 808 that is out of key after a pitch drop.
- A click at every note when the sine starts at a random phase, or a thump that can't be turned off.
- Bass that disappears on phones; distortion that adds fizz instead of the second and third harmonics.
- Glide that retriggers the envelope, or no way to choose which held note wins (the Minimoog's low-note priority makes very fast lines hard; SOS).
- Detuned oscillators that cancel the sub, so the line pumps in level (Attack Magazine).
- Aliasing from drive at the top of the keyboard.

## What MPFree has today

- **Poly** plays mono or legato with a note stack (last note wins) and one glide time, has a square sub an octave down, a zero-delay-feedback ladder, Drive before the filter and Distortion after it. Its presets Grit Bass and Sub Bass are basses.
- **Squelch** is the acid bass.
- **Drums** has a kick with Sweep and Sweep time, a Decay up to 5 s and Drive, and a Boom Kit of long 808-style booms, but a kit is pitched per pad, so it doesn't play a bass line from the pads chromatically, and drums ignore tunings.
- Missing: a sub that follows the main oscillator at two octaves down or as a narrow pulse, a Fifth, a start phase, note priority, a constant-rate glide, an 808 body playable as a chromatic mono instrument with an exact settle, and a low-pass after the drive.

## Sources

- K. J. Werner, J. S. Abel, J. O. Smith, "A physically-informed, circuit-bendable, digital model of the Roland TR-808 bass drum circuit", DAFx 2014: https://dafx14.fau.de/papers/dafx14_kurt_james_werner_a_physically_informed,_ci.pdf
- A. Huovilainen, "Non-linear digital implementation of the Moog ladder filter", DAFx 2004: https://dafx.de/paper-archive/2004/P_061.PDF
- Oyama, DAFx 2026, transistor ladder implementations against SPICE: https://www.dafx.de/paper-archive/2026/papers/DAFx26_paper_29.pdf
- MusicTech, "Make subs that cut through on small speakers": https://www.musictech.com/tutorials/subs-for-small-speakers/
- Violet Recording, "How to design an 808 from scratch": https://violetrecording.com/how-to-design-an-808/
- Attack Magazine, "Reese bass redux": https://www.attackmagazine.com/technique/tutorials/reese-bass-redux/
- Sound On Sound, "Duophony" (note priority and triggering): https://www.soundonsound.com/techniques/duophony
- Roland SH-101 owner's manual, source mixer: https://www.manualslib.com/manual/2211414/Roland-Sh-101.html?page=6
- Glide, legato and note priority as players ask for them (a user's feature proposal, not a spec): https://forum.loopypro.com/discussion/68679/proper-implementation-mono-portamento-glide-in-synths
- MPC Standalone OS 3.6 user guide, Bassline, pp. 400 to 402 (project files)
