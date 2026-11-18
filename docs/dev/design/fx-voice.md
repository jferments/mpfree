<!--
covers: rust/mpfree-fx/src/wah.rs
checked: 2026-11-18 ffa3b26c
-->
# Voice: talk box

**Status:** built (the MPC coverage round, 14 Nov), and rebuilt by FX quality round 5's PR 7 (18 Nov): every voice target met (54 of 54; 29 before). Its research, targets and measuring are round 5's (plan-5 section 11). PR 7 makes the formant bank Klatt's parallel one with alternating signs and fits its levels to Peterson and Barney's on a saw, puts each vowel at the dry's loudness, keeps the level as Resonance sharpens the formants, scales the bandwidths with Formant, and glides every change. A new Vocoder, round 5's PR 8 (decision 12.1), joins this page when it is built. Below, "before PR 7" describes what was there before.

This page is the design of MPFree's voice effects as one effect type: what they are for, the ideas underneath them, the research behind ours, which algorithms each uses and why, how quality and cost are measured, and how the defaults and presets were chosen. The parameter-by-parameter summary lives in [fx.md](fx.md); the measured targets and their history in [fx-quality.md](fx-quality.md#round-5-movement-pitch-and-voice). The research notes, plan and baseline probe are in jferments/mpfree-claude `fx/quality/` (research-voice.md, plan-5.md, baseline-r5-2026-11-17.md, tools/r5_baseline.rs), and the filter bank prototype in `fx/quality/tools/talkbox_proto.py`.

| Effect | Node id | Code | One line |
|---|---|---|---|
| Talk box | `mpfree.fx.talkbox` | [`wah.rs`](../../../rust/mpfree-fx/src/wah.rs) | A sound shaped into vowels by three formant filters, moved by the playing or an LFO |

The talk box shares the auto wah's state-variable filter (`Sv`, with its gliding coefficients) and the movement effects' LFO (`kit::Lfo`); both are described in [fx-movement.md](fx-movement.md).

## 1. What they are for

| Effect | What it does | Jobs |
|---|---|---|
| Talk box (the hardware) | An instrument's sound goes up a tube into the player's mouth and a microphone picks it up, so the mouth shapes it into vowels and consonants | "Talking" guitar and synth leads (the Heil Talk Box, 1973; Roger Troutman's funk), robotic choruses |
| Vowel or formant filter (what MPFree's Talk box is, as the MPC's AIR Talkbox) | Resonances at a vowel's formants, moved by a knob, an envelope or an LFO | A synth or a guitar that says "yeah", "wow" or "ooh" with no voice; formant movement on drums and pads |
| Vocoder (PR 8) | A voice's level in each band sets the level of the same band of another sound | Robot voices, choirs from a synth chord, talking drums, whispers from noise |

## 2. Principles

**A vowel is its formants.** The vocal tract's resonances, F1 and F2 above all, are what make "ah" an "ah" (Peterson and Barney, 1952). The table is their averages for adult men, from OO ("who'd", 300, 870 and 2240 Hz) to EE ("heed", 270, 2290 and 3010 Hz), ordered back to front so neighbours sound alike and a sweep passes through them.

**A mouth has no notches between its formants.** Fant's acoustic theory treats the tract as an all-pole filter: in a cascade of resonators the response between two formants dips gently. Band-passes summed in parallel with the same sign cancel there instead: just above one formant its band-pass lags by a quarter cycle, just below the next its band-pass leads by one, so between them the two are opposite and subtract. The result is hollow and phasey. Klatt (1980) uses a parallel bank for consonants with **alternating signs** (F1 +, F2 -, F3 +): the sign flip turns the cancellation into addition, and the dips come back to about a cascade's.

**On a saw, the levels are the mouth's.** Speech is a glottal source falling about 12 dB an octave, through the tract, out of the lips (about +6 dB an octave): a source falling 6 dB an octave, much like a saw, so a talk box fed a saw should give the formant levels of a sung or spoken vowel. A parallel bank sets each formant's level by hand, so they are fitted to that: on a 110 Hz saw, F2 and F3 come out under F1 by Peterson and Barney's sung levels. Levels set once for a flat input are wrong on a saw, and "EE" turns into "OO" when its F2 is too weak.

**A narrower band passes less.** A band-pass's power on a broad sound follows its bandwidth, so the bank's level falls about 3 dB each time Resonance halves the bandwidths, unless its gain rises with the square root of the sharpness (as the auto wah's does with Q).

**A vowel sweep shouldn't pump.** Each vowel passes a different share of a sound (an "OO" passes little above 1 kHz, an "EE" mostly what lies around 2.3 to 3 kHz), so a sweep along the row changes the level unless each vowel has its own make-up gain, glided with the vowel.

**A filter that moves must glide.** Moving resonant filters every few frames without smoothing their settings zips; a vowel that steps clicks. The coefficients glide from one retune to the next, and the controls glide over 20 ms.

## 3. Research

research-voice.md has the detail and the sources. In short:

- **Formants:** Peterson and Barney's 1952 averages are the usual table; their table II gives the formants' levels, which the measuring PR checked fxlab's table against. Fant (1960) for the all-pole tract; Klatt (1980) for the cascade and parallel banks and their signs.
- **Familiar vowel filters** offer the vowel (a row to sweep), a formant shift (a bigger or smaller mouth), resonance, an envelope and an LFO moving it, and Mix; some add a second vowel to morph to.
- **Pitfalls named by the research:** same-sign parallel band-passes (deep notches), hand-set levels for a flat input (wrong on a saw, and vowels that change loudness), retuning in steps (zipper).

## 4. The engines

### Talk box

Three state-variable band-passes in parallel at the vowel's F1, F2 and F3, summed with alternating signs, each scaled to its level and the vowel's make-up. The envelope (Env depth, Sensitivity, Attack, Release) and the LFO (rate or sync, shape, LFO depth) move the vowel along the row, up to four vowels either way; Vowel sets where it rests. Between two vowels the formants glide in pitch and the levels in dB.

- **The bank** (PR 7): F1 +, F2 -, F3 +, each band-pass normalised to unity at its peak, then scaled by its level. The levels (F2 and F3 in dB against F1, per vowel) come from a fit: on a 110 Hz saw at 44.1 kHz, the harmonic nearest each formant under the one nearest F1 lands on Peterson and Barney's sung levels (fxlab's V3 table). The fit (`talkbox_proto.py` in mpfree-claude) iterates the two gains against the digital response; the dips between F1 and F2 then come out within 1.7 dB of a cascade's on every vowel, or shallower ("IH", whose formants are far apart, 6.4 dB shallower).
- **The make-up** (PR 7): each vowel's own gain (11.3 to 16.2 dB), set so it goes in at the dry's loudness halfway between a pink noise's and a saw's: the nine vowels on the saw come out within 2.0 LU of each other, and the defaults 0.6 LU over the dry on pink (the prototype puts every vowel within -1.5 and +0.6 LU on pink). It glides with the vowel. It replaces the fixed 9.5 dB (`TALK_GAIN` 3.0) of before.
- **Resonance:** half to twice as sharp as speech (bandwidths 80, 100 and 140 Hz at 50 %); the gains rise with the square root of the sharpness, so the level stays put from 0 to 100 (within 0.7 LU on AH in the prototype; the unit test holds it within 3 dB on noise).
- **Formant:** shifts every formant by up to 12 semitones either way, and (PR 7) their bandwidths with them, so a smaller mouth's formants are wider and each formant's Q stays as Resonance sets it. In the prototype the level on pink stays within 1.6 LU of the dry over the whole range.
- **Retuning:** every 16 frames, as the auto wah: the coefficients glide in a straight line to the next retune's (`Sv::glide_to`, `Sv::advance`) and so do the three output gains. Vowel, Formant and Resonance glide over 20 ms, a retune at a time; the vowel position the envelope and LFO move glides over 6 ms, so the talking keeps its pace.

Measured before PR 7 (round 5's baseline, 44.1 kHz): formants within 1.7 % of the table; the dips between F1 and F2 7.1 to 16.3 dB deeper than a cascade's ("AE" the deepest); F2 and F3 off the sung levels by up to 10.1 dB ("EE" F2), 7.3 ("OU" F2), 5.8 ("EE" F3), 5.1 ("AE" F2) and 4.7 ("OO" F3); 6.8 LU between the vowels on the saw; 6.4 LU quieter than the dry on pink (the presets 4.9 to 8.2); Vowel clicked at -38.1 dB, LFO depth -40.8, Formant -42.8, Env depth -43.4 and Resonance -51.0. After it every voice target is met: formants within 1.7 %; no dip more than 1.7 dB deeper than a cascade's; F2 and F3 within 0.005 dB of the sung levels; the vowels within 2.0 LU on the saw; the defaults 0.6 LU over the dry on pink and every preset within 0.8; every control -67 dB or under (Vowel -69.4, LFO depth -67.0).

## 5. Controls

| Effect | Controls (in id order) |
|---|---|
| Talk box | Vowel (OO, OU, AW, AH, UH, AE, EH, IH, EE), Formant (±12 semitones), Resonance, Env depth, Sensitivity, Attack, Release, LFO rate, LFO sync, LFO shape, LFO depth, Mix |

PR 7 changes no control and no id, so saved projects open as they were. They sound as described above: a saved talk box goes in at the dry's level, about 6 LU louder than it was (5 to 8 LU in the presets), with fuller vowels (no notch between F1 and F2, and "EE" brighter); a Formant shift now widens or narrows the formants with it.

## 6. Defaults and presets

- **Talk box** on AH, Resonance 50 %, no movement, Mix 100: a steady vowel at the dry's level.
- Presets: Yeah (EE, the envelope moving it toward OO as it gets louder), Wow wow (OO, a synced LFO), Robot choir (AH, Resonance 85, a slow LFO), Small mouth (AE, Formant +5). Each goes in within 0.8 LU of the dry on pink.

## 7. Measuring quality

fxlab's `voice` class (`quality/round5.rs`, ids V) measures it, and the ratchet holds every number ([fx-quality.md](fx-quality.md#round-5-movement-pitch-and-voice) has the targets and the history):

- **Formants** (V1): each vowel's peaks near F1, F2 and F3 in its frequency response, against the table (within 3 %).
- **Dips** (V2): the lowest point between the found F1 and F2 under the F2 peak, against an all-pole cascade's with the same formants and bandwidths (no more than 3 dB deeper).
- **Levels** (V3): on a 110 Hz saw, the harmonics nearest each formant, F2 and F3 under F1, against Peterson and Barney's sung levels (within 4 dB).
- **Level** (V4): the nine vowels' loudness on the saw (a spread of 4 LU at most); the defaults and every preset on pink (within ±2 LU of the dry).
- **Clicks** (V5): round 3's M13, each control swept end to end in 50 ms and back under a 997 Hz tone (≤ -60 dB).
- Unit tests in `wah.rs`: AH and EE each pass their own formants; Formant moves every formant up an octave and keeps its Q; halfway between two vowels is between them in pitch; "AE" dips no more than 20 dB under F2 between its first two formants (31 dB with same-sign band-passes); every vowel within 6 dB of the defaults on white noise, and Resonance 0 and 100 within 3 dB.

## 8. Cost

The instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz, at the defaults: Talk box 37.3k before round 5, 30.2k after PR W's LFO, 31.3k after PR 7 (+3.6 %: the ramps of the three filters' coefficients and gains, twelve adds a frame). As ARMv7 code (counted under qemu-arm as [fx.md](fx.md) describes), 38.1k before PR 7.

## 9. Pitfalls and how the design avoids them

- **Same-sign parallel band-passes** (before PR 7): notches 7 to 16 dB deeper than a mouth's between F1 and F2. The signs alternate.
- **Levels set for a flat input** (before PR 7, "after Fant"): "EE"'s F2 10 dB weak on a saw. The levels are fitted on a saw.
- **One make-up gain for every vowel** (before PR 7): 6.8 LU between the vowels, and 6.4 LU quieter than the dry. Each vowel has its own, glided.
- **Bandwidths fixed in hertz while Formant shifts** (before PR 7): a bigger mouth's formants relatively wider and a smaller one's sharper. The bandwidths scale with the shift.
- **Coefficients that step at each retune and controls read at once** (before PR 7): Vowel clicked at -38 dB. The coefficients and gains glide between retunes and the controls over 20 ms.
- **A white-noise level check** for a formant filter: white noise favours the front vowels' higher formants, about 5 dB between OO and AH; fxlab reads level on pink, K-weighted.

## 10. History and next

- The MPC coverage round (14 Nov): the talk box (`wah.rs`), after the auto wah.
- FX quality round 5 (17 Nov): research, baseline and plan (mpfree-claude #211, plan-5.md); the measuring PR (#510): 29 of 54 voice targets met. PR W (17 Nov) gave it the shared LFO's fixes.
- PR 7 (18 Nov): the alternating-sign bank with fitted levels, a make-up per vowel, level held across Resonance, bandwidths scaled with Formant, and every change glided: 54 of 54.
- Next: the Vocoder (PR 8, decision 12.1), on this page; pitch correct's MIDI mode is on [fx-pitch.md](fx-pitch.md)'s list.

## 11. Sources

- G. E. Peterson and H. L. Barney, "Control methods used in a study of the vowels", JASA 24, 1952 (the formants, and table II's levels).
- G. Fant, *Acoustic Theory of Speech Production*, Mouton, 1960 (the all-pole tract).
- D. H. Klatt, "Software for a cascade/parallel formant synthesizer", JASA 67, 1980 (the parallel bank's alternating signs).
- A. Simper, "Linear trapezoidal integrated state variable filter" (Cytomic, 2013), the filter the bank is built from.
