<!--
covers: rust/mpfree-fx/src/wah.rs rust/mpfree-fx/src/vocoder.rs
checked: 2026-11-18 8afa51c9
-->
# Voice: talk box and vocoder

**Status:** built (the MPC coverage round, 14 Nov), and rebuilt by FX quality round 5's PR 7 (18 Nov): every voice target met (54 of 54; 29 before). Its research, targets and measuring are round 5's (plan-5 section 11). PR 7 makes the formant bank Klatt's parallel one with alternating signs and fits its levels to Peterson and Barney's on a saw, puts each vowel at the dry's loudness, keeps the level as Resonance sharpens the formants, scales the bandwidths with Formant, and glides every change. PR 8 (18 Nov, decision 12.1) adds the Vocoder, with every one of its targets met (V6). Below, "before PR 7" describes what was there before.

This page is the design of MPFree's voice effects as one effect type: what they are for, the ideas underneath them, the research behind ours, which algorithms each uses and why, how quality and cost are measured, and how the defaults and presets were chosen. The parameter-by-parameter summary lives in [fx.md](fx.md); the measured targets and their history in [fx-quality.md](fx-quality.md#round-5-movement-pitch-and-voice). The research notes, plan and baseline probe are in jferments/mpfree-claude `fx/quality/` (research-voice.md, plan-5.md, baseline-r5-2026-11-17.md, tools/r5_baseline.rs), the talk box's filter bank prototype in `fx/quality/tools/talkbox_proto.py`, and the vocoder's bank sums in `fx/quality/tools/vocoder_bank.py`.

| Effect | Node id | Code | One line |
|---|---|---|---|
| Talk box | `mpfree.fx.talkbox` | [`wah.rs`](../../../rust/mpfree-fx/src/wah.rs) | A sound shaped into vowels by three formant filters, moved by the playing or an LFO |
| Vocoder | `mpfree.fx.vocoder` | [`vocoder.rs`](../../../rust/mpfree-fx/src/vocoder.rs) | A voice's spectrum laid on a saw that follows its pitch, on noise, or on the track's own sound |

The talk box shares the auto wah's state-variable filter (`Sv`, with its gliding coefficients) and the movement effects' LFO (`kit::Lfo`); both are described in [fx-movement.md](fx-movement.md). The vocoder runs its own bank of the same filter, four bands at a time, and follows the voice's pitch with pitch correct's tracker (`vocal.rs`'s `Tracker`, [fx-pitch.md](fx-pitch.md)).

## 1. What they are for

| Effect | What it does | Jobs |
|---|---|---|
| Talk box (the hardware) | An instrument's sound goes up a tube into the player's mouth and a microphone picks it up, so the mouth shapes it into vowels and consonants | "Talking" guitar and synth leads (the Heil Talk Box, 1973; Roger Troutman's funk), robotic choruses |
| Vowel or formant filter (what MPFree's Talk box is, as the MPC's AIR Talkbox) | Resonances at a vowel's formants, moved by a knob, an envelope or an LFO | A synth or a guitar that says "yeah", "wow" or "ooh" with no voice; formant movement on drums and pads |
| Vocoder | A voice's level in each band sets the level of the same band of another sound (the carrier) | Robot voices, choirs from a synth chord, talking drums, whispers from noise |

## 2. Principles

**A vowel is its formants.** The vocal tract's resonances, F1 and F2 above all, are what make "ah" an "ah" (Peterson and Barney, 1952). The table is their averages for adult men, from OO ("who'd", 300, 870 and 2240 Hz) to EE ("heed", 270, 2290 and 3010 Hz), ordered back to front so neighbours sound alike and a sweep passes through them.

**A mouth has no notches between its formants.** Fant's acoustic theory treats the tract as an all-pole filter: in a cascade of resonators the response between two formants dips gently. Band-passes summed in parallel with the same sign cancel there instead: just above one formant its band-pass lags by a quarter cycle, just below the next its band-pass leads by one, so between them the two are opposite and subtract. The result is hollow and phasey. Klatt (1980) uses a parallel bank for consonants with **alternating signs** (F1 +, F2 -, F3 +): the sign flip turns the cancellation into addition, and the dips come back to about a cascade's.

**On a saw, the levels are the mouth's.** Speech is a glottal source falling about 12 dB an octave, through the tract, out of the lips (about +6 dB an octave): a source falling 6 dB an octave, much like a saw, so a talk box fed a saw should give the formant levels of a sung or spoken vowel. A parallel bank sets each formant's level by hand, so they are fitted to that: on a 110 Hz saw, F2 and F3 come out under F1 by Peterson and Barney's sung levels. Levels set once for a flat input are wrong on a saw, and "EE" turns into "OO" when its F2 is too weak.

**A narrower band passes less.** A band-pass's power on a broad sound follows its bandwidth, so the bank's level falls about 3 dB each time Resonance halves the bandwidths, unless its gain rises with the square root of the sharpness (as the auto wah's does with Q).

**A vowel sweep shouldn't pump.** Each vowel passes a different share of a sound (an "OO" passes little above 1 kHz, an "EE" mostly what lies around 2.3 to 3 kHz), so a sweep along the row changes the level unless each vowel has its own make-up gain, glided with the vowel.

**A filter that moves must glide.** Moving resonant filters every few frames without smoothing their settings zips; a vowel that steps clicks. The coefficients glide from one retune to the next, and the controls glide over 20 ms.

**A vocoder is two filter banks and a follower per band.** The voice (the modulator) and the carrier go through the same bands; each voice band's level, followed with an attack and a release, sets its carrier band's level. Speech needs about 16 bands over 100 Hz to 8 kHz to stay intelligible; fewer sound vintage, more add detail. The carrier has to have something in every band: a bright saw, noise, or a full synth chord.

**Neighbouring band-passes cancel where they cross.** The same quarter-cycle argument as the talk box's: a 4th-order band-pass leads or lags by about half a cycle at the frequency where it crosses its neighbour, so bands summed with the same sign cut a notch at every crossing (10 to 40 dB deep for 4 to 40 bands). Alternating signs make them add, and the summed bank is flat within about 1.5 dB when the bands cross 3 dB down.

**A level follower ripples at twice the frequency it follows.** A rectified band ripples at twice its frequency and a fast follower lets much of that through: a low band buzzes, and the ripple's harmonics put sidebands far above the band. The band-pass and a partner 90 degrees from it give the band's envelope without the ripple.

**Consonants have little to follow.** An "s" or a "t" is noise above 4 kHz, where a saw or a synth has little, so a vocoder mixes the voice's own highs back in (Unvoiced).

## 3. Research

research-voice.md has the detail and the sources. In short:

- **Formants:** Peterson and Barney's 1952 averages are the usual table; their table II gives the formants' levels, which the measuring PR checked fxlab's table against. Fant (1960) for the all-pole tract; Klatt (1980) for the cascade and parallel banks and their signs.
- **Familiar vowel filters** offer the vowel (a row to sweep), a formant shift (a bigger or smaller mouth), resonance, an envelope and an LFO moving it, and Mix; some add a second vowel to morph to.
- **Pitfalls named by the research:** same-sign parallel band-passes (deep notches), hand-set levels for a flat input (wrong on a saw, and vowels that change loudness), retuning in steps (zipper).
- **Vocoders:** vintage hardware had 8 to 12 bands, software 20 (Logic's EVOC 20) to 40 (Ableton's) and more; 4 to 6 sound vintage, over 16 add detail at a cost. Each band's envelope has an attack and a release, fast for speech. Unvoiced sounds have little for the bands to follow, so vocoders add the voice's highs back (Ableton's Unvoiced, EVOC 20's U/V detection). A bright carrier works best (a saw, two saws an octave apart, noise for a whisper). Formant shift moves the carrier's bands against the voice's; bandwidth sets how much the bands overlap; the range fits a lower or a higher voice. Carriers offered: another track (a sidechain), noise, the voice itself, or an oscillator that follows the voice's pitch.
- **Vocoder pitfalls:** envelopes too slow (mushy speech) or too fast (a buzz at the voice's pitch), bands so wide the formants smear, and no unvoiced path (the "s" and "t" lost).

## 4. The engines

### Talk box

Three state-variable band-passes in parallel at the vowel's F1, F2 and F3, summed with alternating signs, each scaled to its level and the vowel's make-up. The envelope (Env depth, Sensitivity, Attack, Release) and the LFO (rate or sync, shape, LFO depth) move the vowel along the row, up to four vowels either way; Vowel sets where it rests. Between two vowels the formants glide in pitch and the levels in dB.

- **The bank** (PR 7): F1 +, F2 -, F3 +, each band-pass normalised to unity at its peak, then scaled by its level. The levels (F2 and F3 in dB against F1, per vowel) come from a fit: on a 110 Hz saw at 44.1 kHz, the harmonic nearest each formant under the one nearest F1 lands on Peterson and Barney's sung levels (fxlab's V3 table). The fit (`talkbox_proto.py` in mpfree-claude) iterates the two gains against the digital response; the dips between F1 and F2 then come out within 1.7 dB of a cascade's on every vowel, or shallower ("IH", whose formants are far apart, 6.4 dB shallower).
- **The make-up** (PR 7): each vowel's own gain (11.3 to 16.2 dB), set so it goes in at the dry's loudness halfway between a pink noise's and a saw's: the nine vowels on the saw come out within 2.0 LU of each other, and the defaults 0.6 LU over the dry on pink (the prototype puts every vowel within -1.5 and +0.6 LU on pink). It glides with the vowel. It replaces the fixed 9.5 dB (`TALK_GAIN` 3.0) of before.
- **Resonance:** half to twice as sharp as speech (bandwidths 80, 100 and 140 Hz at 50 %); the gains rise with the square root of the sharpness, so the level stays put from 0 to 100 (within 0.7 LU on AH in the prototype; the unit test holds it within 3 dB on noise).
- **Formant:** shifts every formant by up to 12 semitones either way, and (PR 7) their bandwidths with them, so a smaller mouth's formants are wider and each formant's Q stays as Resonance sets it. In the prototype the level on pink stays within 1.6 LU of the dry over the whole range.
- **Retuning:** every 16 frames, as the auto wah: the coefficients glide in a straight line to the next retune's (`Sv::glide_to`, `Sv::advance`) and so do the three output gains. Vowel, Formant and Resonance glide over 20 ms, a retune at a time; the vowel position the envelope and LFO move glides over 6 ms, so the talking keeps its pace.

Measured before PR 7 (round 5's baseline, 44.1 kHz): formants within 1.7 % of the table; the dips between F1 and F2 7.1 to 16.3 dB deeper than a cascade's ("AE" the deepest); F2 and F3 off the sung levels by up to 10.1 dB ("EE" F2), 7.3 ("OU" F2), 5.8 ("EE" F3), 5.1 ("AE" F2) and 4.7 ("OO" F3); 6.8 LU between the vowels on the saw; 6.4 LU quieter than the dry on pink (the presets 4.9 to 8.2); Vowel clicked at -38.1 dB, LFO depth -40.8, Formant -42.8, Env depth -43.4 and Resonance -51.0. After it every voice target is met: formants within 1.7 %; no dip more than 1.7 dB deeper than a cascade's; F2 and F3 within 0.005 dB of the sung levels; the vowels within 2.0 LU on the saw; the defaults 0.6 LU over the dry on pink and every preset within 0.8; every control -67 dB or under (Vowel -69.4, LFO depth -67.0).

### Vocoder

The voice goes through a bank of 4 to 40 band-passes (16 at the defaults) spaced evenly in pitch from Low to High (100 Hz to 8 kHz); each band's power, followed with Attack and Release, sets the level of the same band of the carrier, and the carrier's bands are summed with alternating signs. The voice's highs above 4 kHz are mixed in (Unvoiced); a gate fades the output while the voice is under its threshold.

- **Carrier:** Saw (default), a saw that follows the voice's pitch (pitch correct's tracker on its Mid range, 90 to 900 Hz; 110 Hz until it finds one), Pitch semitones away; Noise, white noise whose bands are tilted so it plays as pink noise would; or Input, the track's own sound, with the voice on its sidechain key. With Saw or Noise the voice is the track's own sound, so it works on a voice track alone; Input needs a key, which a project sets today (the mixer has no key picker yet), and is silent without one (the Voice level meter reads -60 dB). The internal carriers play at -18 dBFS RMS. The voice and the carrier are mono; Spread makes the output stereo.
- **Bands:** each band is two identical trapezoidal state-variable band-passes in series (4th order), each unity at its peak, all with the same Q: 0.6436 over the half powers of the spacing (r^(b/2) - r^(-b/2), with r the ratio between bands and b Bandwidth), so the stages are 1.5 dB down halfway to the next band and the pair crosses its neighbour 3 dB down at Bandwidth 100 %, where the alternating sum is flattest (within 1.4 dB from 8 to 40 bands; same-sign it would have notches 10 to 40 dB deep). Bandwidth runs from 25 % (bands with gaps between them, a ringing, metallic sound) to 150 % (wider than that, alternating bands cancel where they overlap). Formant moves the carrier's bands up to 12 semitones against the voice's (a smaller or a bigger mouth); bands moved past 0.405 of the sample rate fade out rather than pile up at the top.
- **Envelopes:** each band's power is its band-pass squared plus a partner 90 degrees from it, half the second stage's low-pass less its high-pass, which is in quadrature with the band-pass at every frequency and within 1 % of its size across the band. A tone in the band reads a steady power instead of a rectified wave at twice its frequency, so low bands don't buzz and no sidebands land far above the band. The power is followed with Attack and Release (a power's time constants are half its level's, so the controls read as the level's), and its square root sets the band's level.
- **Level:** each band's gain is the square root of the range in octaves times the bands' Q, over -18 dBFS and over the square root of the bank's summed power at that Bandwidth (tabulated, as it hardly depends on the bands' number or range). A band passes about 1.13 / Q octaves of the voice, so its envelope goes as the square root of that; the carrier comes through the bank as its summed power says. A voice at -18 dBFS RMS then gives back about the carrier's loudness, whatever Bands, Low, High and Bandwidth are (the constant, 0.607, is fitted on sung vowels; the theory gives 0.66).
- **Unvoiced:** the voice through a 4th-order Butterworth high-pass at 4 kHz, at Unvoiced's level (70 %, about -3 dB, by default).
- **Gate:** the voice's power, followed fast (1 ms up, 40 ms down), opens it at the threshold and closes it 6 dB under; the gate's gain rises in 1 ms and falls in 30 ms. Fully down (-90 dB) it is off.
- **Gliding:** Low, High, Formant, Bandwidth and Spread glide over 20 ms, a retune (16 frames) at a time; the bands' coefficients and gains ramp in a straight line from one retune to the next, as the talk box's, and stop ramping once everything has landed. The saw's pitch glides over 8 ms. Carrier and Bands change the bands' layout: the output dips out over 6 ms, the bands are laid out afresh, and it comes back over 6 ms (a smoothstep both ways).

## 5. Controls

| Effect | Controls (in id order) |
|---|---|
| Talk box | Vowel (OO, OU, AW, AH, UH, AE, EH, IH, EE), Formant (±12 semitones), Resonance, Env depth, Sensitivity, Attack, Release, LFO rate, LFO sync, LFO shape, LFO depth, Mix |
| Vocoder | Carrier (Saw, Noise, Input), Pitch (±24 semitones, Saw only), Bands (4 to 40), Low (50 to 1000 Hz), High (1 to 16 kHz), Formant (±12 semitones), Bandwidth (25 to 150 %), Attack (0.5 to 100 ms), Release (5 to 1000 ms), Gate (-90 to -20 dB, off fully down), Unvoiced, Spread, Mix; a meter, Voice level. Its page: Carrier, Bands, Voice and Output sections; Main's four (the Q-Links) Formant, Bandwidth, Release and Mix; Pitch dimmed unless the carrier is Saw. The hint line says what each carrier does while Carrier is touched |

PR 7 changes no control and no id, so saved projects open as they were. They sound as described above: a saved talk box goes in at the dry's level, about 6 LU louder than it was (5 to 8 LU in the presets), with fuller vowels (no notch between F1 and F2, and "EE" brighter); a Formant shift now widens or narrows the formants with it.

## 6. Defaults and presets

- **Talk box** on AH, Resonance 50 %, no movement, Mix 100: a steady vowel at the dry's level.
- Presets: Yeah (EE, the envelope moving it toward OO as it gets louder), Wow wow (OO, a synced LFO), Robot choir (AH, Resonance 85, a slow LFO), Small mouth (AE, Formant +5). Each goes in within 0.8 LU of the dry on pink.
- **Vocoder** on the Saw carrier, 16 bands from 100 Hz to 8 kHz at Bandwidth 100 %, Attack 3 ms, Release 40 ms, Gate -70 dB, Unvoiced 70 %, Mix 100: a robot voice at the voice's pitch and level, from a voice track alone, with no routing to set up. Input would be the classic synth-keyed vocoder, but it needs a key the app can't pick yet, and silent by default would look broken.
- Presets (all on internal carriers, so none needs a key): Robot voice (12 bands, Release 30), Deep robot (an octave down, Low 50 Hz so the bands reach the saw, Formant -3), Whisper (Noise, 24 bands, Unvoiced 100), Vintage bands (8 bands from 120 Hz to 5 kHz, Bandwidth 140 %), Wide choir (24 bands, Release 90, Spread 80). Each is within 1.2 LU of the voice on pink.

## 7. Measuring quality

fxlab's `voice` class (`quality/round5.rs`, ids V) measures it, and the ratchet holds every number ([fx-quality.md](fx-quality.md#round-5-movement-pitch-and-voice) has the targets and the history):

- **Formants** (V1): each vowel's peaks near F1, F2 and F3 in its frequency response, against the table (within 3 %).
- **Dips** (V2): the lowest point between the found F1 and F2 under the F2 peak, against an all-pole cascade's with the same formants and bandwidths (no more than 3 dB deeper).
- **Levels** (V3): on a 110 Hz saw, the harmonics nearest each formant, F2 and F3 under F1, against Peterson and Barney's sung levels (within 4 dB).
- **Level** (V4): the nine vowels' loudness on the saw (a spread of 4 LU at most); the defaults and every preset on pink (within ±2 LU of the dry).
- **Clicks** (V5): round 3's M13, each control swept end to end in 50 ms and back under a 997 Hz tone (≤ -60 dB).
- **The vocoder** (V6): keyed through its input, a 110 Hz saw carrier and a sung vowel at 220 Hz on the key, both at -18 dBFS RMS: the output's loudness against the saw's for each of the nine vowels (within ±2 LU); near F1 and near F2, where the voice is loudest and where the output's gain over the saw peaks, read on the saw's harmonics, and how many bands apart (within one); the defaults and the presets on pink at the track (within ±2 LU of it); an "s" (white noise above 4 kHz at -18 dBFS) on the track and on the key, its level out against in (no more than 6 dB down); the latency (0); and the click measure with the 997 Hz tone as voice and carrier both (the node keyed by its own input), every control but Carrier (≤ -60 dB).
- Unit tests in `vocoder.rs`: a keyed 1 kHz tone brings out the noise carrier's band at 1 kHz and not at 4 kHz; Formant +12 moves it to 2 kHz; Input with no key is silent and the meter reads -60 dB; the saw speaks noise at -18 dBFS within 6 dB of its level at 4, 16 and 40 bands, Bandwidth 40 % and a narrow range; the gate shuts a quiet voice out, and fully down lets it through; Spread parts the channels and none leaves them identical; changing Bands dips and comes back to the same level.
- Unit tests in `wah.rs`: AH and EE each pass their own formants; Formant moves every formant up an octave and keeps its Q; halfway between two vowels is between them in pitch; "AE" dips no more than 20 dB under F2 between its first two formants (31 dB with same-sign band-passes); every vowel within 6 dB of the defaults on white noise, and Resonance 0 and 100 within 3 dB.

## 8. Cost

The instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz, at the defaults: Talk box 37.3k before round 5, 30.2k after PR W's LFO, 31.3k after PR 7 (+3.6 %: the ramps of the three filters' coefficients and gains, twelve adds a frame). As ARMv7 code (counted under qemu-arm as [fx.md](fx.md) describes, 48 kHz, white noise in), 38.1k before PR 7 and 44.0k after it (+15.6 %: the ramps cost more as scalar code than on x86, where they vectorise).

The Vocoder (PR 8), the gate's case at its defaults (16 bands, the saw following a 110 Hz tone with noise bursts): 111.2k on x86. As ARMv7 code on white noise (the tracker's full search every 5 ms, its worst case): 273.6k at 16 bands, 589.1k at 40, and 237.8k on Input (no tracker, no saw). Each band costs about 13k a block (about 100 instructions a frame: four filter stages, the follower and the square root, the two output gains), the rest about 64k (the tracker and saw about 36k, the gate, the high-pass and the meter). On x86 the bands run four at a time (2.6 times fewer instructions than one at a time); the ARMv7 build has no NEON, so on the MPC each band is scalar. That puts it among the dearest effects: over Lo-fi (227k) and Drive's Eco 2x (232k), under Drive's HQ 4x (562k) and the Cloud reverb (204k to 1.7M by mode). At about one instruction a cycle at 1.6 GHz (an estimate until a device round measures it), 16 bands are about 6 % of a 128-frame period on one core and 40 bands about 13 %; round 7's budget for one effect was 1 to 6 %. Fewer bands cost proportionally less. The way to cut it further is a multirate bank (the bands under a tenth of the rate run at a half or a quarter of it, about 40 % less at 16 bands), left for a later round with its own quality checks.

## 9. Pitfalls and how the design avoids them

- **Same-sign parallel band-passes** (before PR 7): notches 7 to 16 dB deeper than a mouth's between F1 and F2. The signs alternate.
- **Levels set for a flat input** (before PR 7, "after Fant"): "EE"'s F2 10 dB weak on a saw. The levels are fitted on a saw.
- **One make-up gain for every vowel** (before PR 7): 6.8 LU between the vowels, and 6.4 LU quieter than the dry. Each vowel has its own, glided.
- **Bandwidths fixed in hertz while Formant shifts** (before PR 7): a bigger mouth's formants relatively wider and a smaller one's sharper. The bandwidths scale with the shift.
- **Coefficients that step at each retune and controls read at once** (before PR 7): Vowel clicked at -38 dB. The coefficients and gains glide between retunes and the controls over 20 ms.
- **A white-noise level check** for a formant filter: white noise favours the front vowels' higher formants, about 5 dB between OO and AH; fxlab reads level on pink, K-weighted.
- **A vocoder's bands summed with one sign:** a notch 10 to 40 dB deep at every crossing. Its bands alternate, crossing 3 dB down (flat within 1.4 dB); wider than 150 % they would cancel, so Bandwidth stops there.
- **A rectified envelope:** a ripple at twice the band's frequency, a buzz in the low bands and sidebands far above the band. The band-pass and its quadrature partner give a steady power.
- **A fixed gain per band:** the level moves with Bands, Bandwidth and the range (narrow bands leave gaps, wide ones overlap). The gain follows the bands' Q and range and the bank's summed power at that Bandwidth: the presets and the layouts the unit test tries stay within a few dB of each other.
- **A voice in one band:** the level law assumes a voice spread over the bands, as speech and song are. A bare sine or a whistle puts all its power into one band and comes out up to 7 dB over the carrier (the bench's 110 Hz tone does).
- **An Input carrier with no key:** silence, which looks broken. The default and every preset play from the track's own sound; the Voice level meter and the Carrier hint say when Input has nothing to listen to.

## 10. History and next

- The MPC coverage round (14 Nov): the talk box (`wah.rs`), after the auto wah.
- FX quality round 5 (17 Nov): research, baseline and plan (mpfree-claude #211, plan-5.md); the measuring PR (#510): 29 of 54 voice targets met. PR W (17 Nov) gave it the shared LFO's fixes.
- PR 7 (18 Nov): the alternating-sign bank with fitted levels, a make-up per vowel, level held across Resonance, bandwidths scaled with Formant, and every change glided: 54 of 54.
- PR 8 (18 Nov): the Vocoder (decision 12.1), every V6 target met: 4 to 40 alternating-sign bands, quadrature envelopes, a level that holds as the bands change, Saw, Noise and Input carriers, Unvoiced, a gate and Spread.
- Next: a key picker in the mixer, so Input can be chosen in the app (the mixer's job); a multirate bank if the vocoder's cost on the MPC needs cutting; pitch correct's MIDI mode is on [fx-pitch.md](fx-pitch.md)'s list.

## 11. Sources

- G. E. Peterson and H. L. Barney, "Control methods used in a study of the vowels", JASA 24, 1952 (the formants, and table II's levels).
- G. Fant, *Acoustic Theory of Speech Production*, Mouton, 1960 (the all-pole tract).
- D. H. Klatt, "Software for a cascade/parallel formant synthesizer", JASA 67, 1980 (the parallel bank's alternating signs).
- A. Simper, "Linear trapezoidal integrated state variable filter" (Cytomic, 2013), the filter the bank is built from.
- MusicRadar, "How to get the most out of vocoders: a complete guide" (band counts, envelopes, unvoiced, carriers, formant shift, bandwidth, range); Ableton Live's manual, Vocoder; Apple's Logic Pro manual, EVOC 20 (from reading, as research-voice.md says).
