# String machines and ensemble pads

**Status:** research, 2026-11-18 (synth library stage 1, wave 1). Feeds the string machine engine planned in the synth library plan ("Synth library: stock-take and research plan", https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). Pads made on a poly synth are covered under Poly in the plan; this page is about the 1970s string ensemble sound and the ensemble effect that makes it, which pads on every engine can use. Nothing here has run on the MPC.

Where a point comes from a source it is linked; where it is our own reading, arithmetic or a target we propose, it says so.

## 1. What makes a great one

- **The ensemble, above all.** A string machine's raw tone is a bright, buzzy divided-down wave; what makes it strings is a chorus of several delay lines, each modulated differently, so it "sounds like several signals with different vibratos" ([Florian Anwander, Roland string choruses](https://www.florian-anwander.de/roland_string_choruses/)). In the Solina the sound passes through three delay lines, "each modulated out of phase with the others by a pair of LFOs" ([Sound On Sound, Eminent 310 review](https://www.soundonsound.com/reviews/eminent-310-string-synthesizer)).
- **Lush, not seasick.** The slow LFO sweeps the delay for a chorus; the fast one adds vibrato. Roland's RS-202 paired 0.66 Hz and 6.25 Hz on each of three lines, set 120° apart (Anwander). Spreading the lines across the stereo field (the 310 and later Solinas had three-channel or stereo outputs) and cross-connecting them makes it swirl (SOS).
- **Warm top end.** Early bucket-brigade chips passed only about 6 kHz, which rolled off the highs and made the sound warmer (SOS).
- **Every key, every time.** One master oscillator divided down gives "full polyphony" ([ARP String Ensemble](https://en.wikipedia.org/wiki/ARP_String_Ensemble)), so a player can hold any chord. Each chord note beats slightly against the others (SOS).
- **A soft swell and a long fade.** The Solina had one envelope shared by every voice (paraphonic): a crescendo in, a sustain out (SOS). Its bass registers (Contra-Bass and Cello) were monophonic over the bottom 20 notes.

## 2. How musicians use it

- Held chords under a song, swelling in and fading out; string lines doubled an octave up; the bed under a ballad; the swirl of 1970s and 1980s pop, rock, disco and synth records (SOS and Wikipedia list Pink Floyd, Elton John, the Bee Gees, Jean-Michel Jarre and others).
- Through extra reverb and echo, which "helped the Solina's strings" (SOS).
- Today: lo-fi and retro pads, chord stabs with a slow release, and the ensemble on other sources (an organ through it, a poly pad through it). Jarre's sound came from sustained strings with a chorused organ stop (SOS).

## 3. Algorithms

- **Divide-down.** Twelve tone generators, one per note of the top octave, divided by two for every octave below ("twelve discrete tone generators with octave divide-down", Wikipedia; the Eminent 310 used one master oscillator divided to the top octave, SOS). The saw-like wave comes from a filtered and rectified divider square ([KVR thread](https://www.kvraudio.com/forum/viewtopic.php?p=3269504), Jim Y); the Crumar Performer used two per note, 8' and 16' (same thread). Our reading: in software, 12 band-limited oscillators at the top octave and phase-locked dividers below cost the same however many keys are held, as Drawbar's always-turning wheels do; each key only opens a gate. A per-key gain with its own swell is cheap.
- **Registers.** Violin, viola, cello, contrabass, horn and trumpet on the Solina (Wikipedia), each a mix of footages through a fixed tone filter (our reading of what a register is on these machines).
- **The ensemble.**
  - Three lines with one slow and one fast LFO each, 120° apart: RS-202, 0.66 and 6.25 Hz (Anwander). The RS-09 and RS-505 used four lines, two of them modulated by a slow LFO alone and two by a slow and a fast one together (0.15, 4.7, 0.18 and 5.9 Hz; 0.17 triangle, 6.25 sine, 0.14 triangle, 5.5 sine).
  - The bucket-brigade line itself: [Raffel and Smith, "Practical modeling of bucket-brigade device circuits", DAFx 2010](https://dafx.de/paper-archive/details/JhVfAOFXD1lAtctkMTUODg) model the device and its filters; [ensemble-chorus](https://github.com/jpcima/ensemble-chorus) models string-ensemble choruses with 3 to 6 lines, each with a slow "chorus" and a fast "vibrato" LFO at fixed phase offsets, citing that paper and Jürgen Haible's Solina emulator. Its licence must be checked before reading its code.
  - MPFree already has a bucket-brigade chorus in Analog (a clock-swept line, Juno-style rates, 9 kHz before and 7.5 kHz after, hiss) and a rebuilt Chorus effect in `mpfree-fx` (FX quality round 3). Our reading: the ensemble is that line, tripled, with the slow and fast LFO pair per line, and belongs in `mpfree-fx` so any track can use it.
- **Delay times.** Not found for the string ensembles yet. The Dimension-D, a related chorus, uses 7.5 to 10 ms with ±1.5 to ±2.5 ms of modulation (Anwander). Stage 1's deep pass looks for a service manual with stage counts and clock rates.

## 4. What to measure

Proposed targets for stage 4 (ours, in brackets, until the baseline and listening confirm them).

| Id | Measure | Proposed target | Why |
|---|---|---|---|
| S1 | Ensemble pitch wobble | Slow and fast components at the set rates within [5] %, depth in cents as labelled | The ensemble is the sound |
| S2 | Line phase | The three lines' modulation [120°] apart | Lush without beating against itself |
| S3 | Stereo | Correlation between [0.2 and 0.6] with the ensemble on; mono fold-down loss under [3] dB | Width that survives mono |
| S4 | Top end | Wet path rolled off from about [6 to 9] kHz | Warmth (SOS) |
| S5 | Cost against keys held | Flat within [10] % from 1 to 49 keys held | Divide-down promise |
| S6 | Tuning | Every key within [1] cent of its tuning, any tuning MPFree offers | Dividers lock to the top octave; non-12-TET tunings need a per-key oscillator fallback (our reading) |
| S7 | Swell and fade | Crescendo and sustain times as labelled within [10] % | Playable swells |
| S8 | Aliasing | Worst alias under [−60] dB at the top octave | Bright waves |
| S9 | Level against chord size | A 6-note chord within [6] dB of a single note's level plus the chord's power sum | No surprise jumps |

## 5. Knobs and defaults

| Section | Controls players expect | Proposed defaults |
|---|---|---|
| Registers | Violin, Viola, Cello, Contrabass, Horn, Trumpet (mix or buttons), Octave | Violin and Viola |
| Tone | Brightness, Body (a fixed formant), Noise (bow) | Middle |
| Swell | Crescendo (attack), Sustain (release), Shared or per key | 400 ms, 1.5 s, per key |
| Ensemble | On, Amount, Slow rate, Fast rate, Width, Mode (three lines; four-line Roland style) | On, 0.66 and 6.25 Hz, wide |
| After | Phaser, Reverb, Delay saved with the preset | Reverb on |

## 6. The presets players expect

A first canon of 24 (our reading): classic strings, bright strings, cello and viola, full ensemble, orchestra (strings plus brass registers), brass ensemble, slow swell strings, staccato strings, phased strings, dark strings (low cutoff), octave strings, strings with chorused organ (the Jarre layering), and pads through the ensemble from other engines (Poly saw pad, Analog warm pad, Wavetable vowel pad).

## 7. Playing it on the MPC

- **Chord pads.** The pads' chord layouts already play three or four notes a pad; a string machine wants them.
- **Pressure** as a swell (the crescendo under the finger) and as the ensemble's depth.
- **Q-Links:** Crescendo, Sustain, Ensemble amount, Brightness.
- **Paraphonic or per key** as a choice: shared swell for the old feel, per key for playing lines.

## 8. What goes wrong

- A chorus that beats audibly or seasickens (one LFO, or all lines in phase).
- Fizz instead of warmth (no roll-off in the wet path).
- Cost that climbs with every key held (per-voice oscillators where a shared bank would do).
- Lost tuning flexibility (dividers lock to 12-TET; MPFree's tunings must still work).
- A wide sound that vanishes in mono.

## What MPFree has today

- **Analog's Chorus Strings preset** (Saw+Pulse, LFO on the width, Fast chorus) and Poly and Wavetable pads.
- **Choruses:** Analog's bucket-brigade chorus and `mpfree-fx`'s Chorus (two-line), both measured or rebuilt in the FX rounds.
- **Missing:** a divide-down engine with registers and swell, and a three- or four-line ensemble effect.

## Sources

- Sound On Sound, Eminent 310 Unique review (and the Solina): https://www.soundonsound.com/reviews/eminent-310-string-synthesizer
- Florian Anwander, Roland choruses and ensemble effects (LFO rates, lines and phases, from service manuals; a work in progress): https://www.florian-anwander.de/roland_string_choruses/
- ARP String Ensemble (Solina), Wikipedia: https://en.wikipedia.org/wiki/ARP_String_Ensemble
- KVR forum, "A chorus like in old stringmachines": https://www.kvraudio.com/forum/viewtopic.php?p=3269504
- C. Raffel, J. O. Smith, "Practical modeling of bucket-brigade device circuits", DAFx 2010: https://dafx.de/paper-archive/details/JhVfAOFXD1lAtctkMTUODg
- jpcima, ensemble-chorus (a digital model of string ensemble chorus): https://github.com/jpcima/ensemble-chorus
- MPC Standalone OS 3.6 user guide, Solina, pp. 426 to 428 (project files)
