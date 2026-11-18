<!--
covers: rust/mpfree-fx/src/pitch.rs rust/mpfree-fx/src/vocal.rs rust/mpfree-fx/src/tuner.rs
checked: 2026-11-18 1a0b4bde
-->
# Pitch: pitch shifter, pitch correct, harmonizer and tuner

**Status:** built (F2 and the vocal effects), and through FX quality round 5: every pitch target met (84 of 84 at 44.1 kHz; 53 of 82 before round 5's fixes). PR 5 (18 Nov) rebuilt the pitch shifter as a WSOLA shifter with aligned splices, a twelve-tap sinc read and a 30 ms default Window. PR 6 (18 Nov) rebuilt pitch correct's and the harmonizer's analysis and grains (a steadier period, looks lined up with the grains, marks and grains between frames read through the same twelve-tap sinc) and added Flex-Tune and Humanize. Below, "before PR 5" and "before PR 6" describe what was there before.

This page is the design of MPFree's pitch effects as one effect type: what they are for, the ideas underneath them, the research behind ours, which algorithms each uses and why, how quality and cost are measured, and how the defaults and presets were chosen. The parameter-by-parameter summary lives in [fx.md](fx.md); the measured targets and their history in [fx-quality.md](fx-quality.md#round-5-movement-pitch-and-voice). The research notes, plan and baseline probe are in jferments/mpfree-claude `fx/quality/` (research-pitch.md, plan-5.md, baseline-r5-2026-11-17.md, tools/r5_baseline.rs).

| Effect | Node id | Code | One line |
|---|---|---|---|
| Pitch shifter | `mpfree.fx.pitch` | [`pitch.rs`](../../../rust/mpfree-fx/src/pitch.rs) | The whole sound up or down, any material, splicing where the waveform lines up |
| Pitch correct | `mpfree.fx.pitch_correct` | [`vocal.rs`](../../../rust/mpfree-fx/src/vocal.rs) | One voice pulled to the notes of a key, formants kept |
| Harmonizer | `mpfree.fx.harmonizer` | `vocal.rs` | Up to two voices a scale's intervals away from the voice |
| Tuner | `mpfree.fx.tuner` | [`tuner.rs`](../../../rust/mpfree-fx/src/tuner.rs) | The note and the cents, passing the sound through or muting it |

## 1. What they are for

| Effect | What it does | Typical settings | Jobs |
|---|---|---|---|
| Pitch shifter | Moves everything, any material | ±12 semitones; a few cents either side for a doubler; Feedback for stairs | Octave layers on bass, guitar and synths, the chipmunk or monster voice, a detuned doubler (Eventide's micro-pitch: ±5 to 15 cents), shimmer |
| Pitch correct | Moves a single voice to the nearest note of a key and scale | Speed 0 (the hard-tune step) to 50 to 100 ms (natural) | Tuning a vocal, the hard-tune effect, tuning a mono synth or bass line |
| Harmonizer | Adds voices a scale interval away | A 3rd and a 5th, an octave; levels under the dry | Backing harmonies from one voice, octave stacks |
| Tuner | Reads the note and cents | Reference A4 = 440 Hz (415 to 466) | Guitars, basses, voices and samples |

The familiar units set what musicians expect: Eventide's H910 (1975) and its de-glitching H949 (1977) for the delay-line shifter, Antares Auto-Tune for pitch correct (Retune Speed, Humanize, Flex-Tune), the vocal harmony boxes, and pedal tuners (±1 cent; strobes a tenth of a cent).

## 2. Principles

**A delay-line shifter is a read that moves.** Write the sound into a line and read it back with a delay that changes by `1 - ratio` a frame: the read then moves at `ratio` times the write, so the pitch is `ratio` times the input's. The delay can't change for ever, so the read jumps back (shifting up, the delay shrinks to nothing) or on (shifting down, it grows past the window) and the old read fades out over the new one. Every artefact a delay-line shifter has is in that splice.

**Splice where the waveform lines up.** A jump of a fixed length lands at whatever phase it lands: on an unlucky note the two reads are out of phase and the crossfade nearly cancels, and the pitch slides by the phase lost each splice. The H949's de-glitch, and WSOLA after it (Verhelst and Roelands 1993), search the jumps near the planned one for the offset where the new read's sound matches what the old read just played, and splice there. On a steady note the two line up and the splice isn't heard.

**What two reads don't share adds in power.** On a chord or noise no offset lines both reads up, and a crossfade of two unrelated sounds dips by up to 3 dB in the middle. The match's normalised correlation ρ says how alike they are, so the crossfade's gain is `1 / sqrt(go² + gn² + 2ρ go gn)` for the old and new gains `go` and `gn`: unity on a perfect match, the power sum on none.

**A moving read must be as bright at any fraction.** Linear interpolation is a low-pass that changes with the fraction (flat at whole frames, -3.9 dB at 15 kHz half way): a read that sweeps through every fraction dulls the highs and buzzes at the rate it sweeps. A windowed sinc with enough taps is flat to the top of the band at every fraction, and its images (what it lets through from above Nyquist after the resampling) are far down.

**Going down leaves images; take them out.** A read that moves at half speed resamples, and what a short interpolator leaves of the input's images lands in the band. A low-pass at the shifted band's top takes out what is left, as a resampler would.

**The delay is half a window, and it is heard.** On average a shifter's read is half its window behind the write. Mixed with the dry (a harmony, an octave layer), a 25 ms lag is a slapback; the shorter the window, the tighter the layer, and aligned splices let the window be short (decision 12.2: immediate, about 15 ms, reported 0).

**Formants move with a delay-line shifter.** It resamples everything, the vocal tract's resonances too: a 5th up moves F1 from 780 to 974 Hz (the chipmunk). Keeping the formants needs the pitch, which is what pitch correct and the harmonizer have (PSOLA grains keep their content and move their spacing).

**A correction is as steady as the pitch it hears.** Pitch correct's ratio is the note less the pitch heard, so every wobble in the tracker is a wobble in the voice, and a ratio that shakes makes every grain a little different: roughness. A look that misses by 3 cents once in ten was enough to bury a corrected saw's harmonics in buzz 48 dB down. The period is found again at the full rate on a smooth (low-passed) copy, where the difference function's dip is a parabola and a parabola through it is right.

**Read the note for the sound being played.** The grains a voice lays now read the input a latency behind, so the note that sets their ratio must be the note of that stretch, not the newest look's. Each look's note is kept with the middle of the stretch it heard and read where the grains read: a vibrato is followed, not chased half a cycle late.

**Every grain reads at its own fraction, so the read must sound the same at all of them.** A grain sits between frames by the mark's fraction less its own, which differs from grain to grain; a straight-line read dulls the top by a different amount at each, and the difference is heard as roughness. The pitch shifter's twelve-tap sinc is flat at every fraction, and a grain takes one set of taps for its whole length.

## 3. Research

research-pitch.md has the detail and the sources. In short:

- **The delay-line shifter** (H910, 1975): two taps crossfaded, the long fade metallic, the short one stuttering. The H949 aligned the new tap by autocorrelation, best on near-periodic monophonic sources and worst on chords and drums (Valhalla DSP). WSOLA (Verhelst and Roelands 1993) is the same idea, written down.
- **PSOLA** (Moulines and Charpentier 1990): for a single voice, grains of two periods at each pitch mark, laid down at the new period; formants stay. Latency about two to three of the lowest periods.
- **The phase vocoder:** good on chords, softens transients, a frame of latency (20 to 90 ms).
- **Auto-Tune:** Retune Speed (0 is the step), Humanize (slower on held notes), Flex-Tune (only near the note, so gestures survive), 58 ms at best quality and under 3 ms in low-latency modes (Sound On Sound's review of AutoTune 2026).
- **Tuners:** ±1 cent is the familiar standard (Boss TU-3: C0 to C8, 436 to 445 Hz).

## 4. The engines

### Pitch shifter

Two voices (left and right), each a delay line read through a twelve-tap sinc, with the search's decimated copy beside it (PR 5):

- **The read** sits `d` frames behind the write and moves `1 - ratio` a frame. On average it lags a quarter of the shortest and longest jumps together (about 11 ms at the default Window), and it starts there.
- **When it splices:** shifting up, once the delay is too short for the old read to fade out before it reaches the write (`d ≤ 7 + (ratio - 1) × fade`); shifting down, once the delay reaches the Window. Each jump is between the Window less the search range and the Window (at the default 30 ms, 15 to 30 ms), and shifting down never past the write.
- **The search**, in three steps: on a copy at an eighth of the rate (each value the mean of 8 frames), every jump in the range against the last 8 ms the read heard, by normalised correlation; then frame by frame, ±8 frames around the best, against the last 4 ms at the full rate; then between frames, the peak of a parabola through the best and its neighbours. The search range is half the Window, at most 25 ms (a period of a bass's low E). It runs about 45 times a second a voice an octave up; its sums are four lanes wide.
- **The fade** is a smoothstep (`t² (3 - 2t)`, flat at both ends) over at most 6 ms (shorter at big shifts, so the old read never reaches the write), with the gain `1 / sqrt(go² + gn² + 2ρ go gn)` from the match.
- **The sinc:** a Kaiser-windowed sinc (β 6, cutoff 0.46 of the rate), 12 taps, 64 fractions with a straight line between neighbours, built at compile time so it is the same everywhere. It passes the lows exactly and puts its images 76 dB down. The line writes every sample twice, a line's length apart, so the twelve taps of any read sit side by side.
- **Going down**, a two-pole low-pass at 0.45 × ratio of the rate fades in from ratio 0.7 (about -6 semitones) to fully on at 0.5 (an octave down).
- **Spread** shifts the left side down and the right up by half of it each, for a doubler. **Feedback** feeds the output, through the soft saturator, back into the line, so each pass is shifted again (stairs); with none the loop is skipped. Ratios, plans and the low-pass are worked out again only when Pitch, Fine, Spread or Window move; every control glides.

Before PR 5 the shifter was the classic two-head design: two reads half a window apart with sin² crossfades, splices at fixed points, linear interpolation, and a 50 ms default window.

### Pitch correct and harmonizer

Both rest on three parts (PR 6 rebuilt the second and third, and the tracker's last step):

- **The tracker:** YIN (de Cheveigné and Kawahara 2002) every 5 ms on the input low-passed and taken down to 6, 8 or 12 kHz for the Low, Mid and High Range; then the period found again at the full rate on the low-passed middle (two one-poles at 800 Hz, the copy marks are found on): the difference function over the newest period (at least 4 ms) from the lag nearest YIN's downhill to its dip, and a parabola through it. A saw from 82 Hz to 1 kHz reads within a hundredth of a frame. The low-passed copy is written twice, a buffer apart, so the refinement reads it in place.
- **The looks:** each look's note sits at the middle of the stretch it heard (a voiced look's, the refinement's window and a period before it; an unvoiced look's, the tracker's whole stretch), sixteen kept. The note is read where the grains laid now read (the latency behind, a period on), on a straight line between voiced looks; from an unvoiced look to a voiced one it is the voiced one's note, so a note's start is corrected with it (Speed 0 settles 4 ms in).
- **The grains (PSOLA):** marks a period apart on the low-passed middle's peaks (every 5 ms while unvoiced), each between frames at the top of a parabola. A voice lays a grain from the mark nearest its target time every local period divided by its ratio, centred between frames; it rises over the gap back to the mark before and falls over the gap on to the mark after in a smootherstep (flat in slope and curve at both ends) and one minus it, so neighbours sum to one, and where faster grains overlap more the windows' sum divides the output back down. A grain reads the input at one fraction (the mark's less its own) through the pitch shifter's twelve-tap sinc: the taps are worked out once a grain and the grain is laid 128 frames at a time, four frames at a time through the taps; on a whole frame it reads the frame itself, so a note held in tune comes out as the input, a latency late, sample for sample. Near ratio 1 a grain that is off its mark eases back, by up to 0.1 % of a period a grain, fully within 0.03 % of 1 and fading out by 0.1 %. The input is written twice, a buffer apart, so each read's taps sit side by side.

**Pitch correct** moves the voice to the nearest note of the key and scale (worked out again only when the pitch leaves the stretch where that note is the nearest) by Amount, through a one-pole whose time constant is Speed: at Speed 200 a note 30 cents off is within 5 cents in 1.79 × 200 ms, as such a one-pole is. **Flex-Tune** corrects only notes sung near the note: the correction fades out between half a reach and the reach, which shrinks from half a semitone to 5 cents as Flex-Tune goes from 0 to 100 (at 0 every note is corrected), so slides and gestures survive. **Humanize** slows the correction on a note held past 100 ms, by up to 400 ms more at 100 % (all of it from 200 ms), so a held note keeps its vibrato while quick notes still snap. A **Range** change dips the output for 5 ms either side of it, since the tracker starts again and the latency changes. The ratio is worked out again only when the correction moves.

**The harmonizer** adds up to two mono voices a scale's intervals from the note sung (each worked out again only when that note changes), each with a level and pan, each through a DC blocker at 5 Hz: shifted up by a ratio between 1 and 2, the grains overlap unevenly over each period and their weighted sum leans to one side (a 3rd up on a saw carried a DC 28 dB under the voice). Its Range change dips the same way.

Both report their latency (two and a half of the Range's longest periods): 41.7, 27.8 and 16.7 ms at 44.1 kHz. [fx.md](fx.md) has every control.

Before PR 6, the marks sat on whole frames, the grains read by a straight line between frames, the note was the newest look's (ahead of the grains by the latency), Speed ran faster than its label (a 30-cent step settled in 25 ms at Speed 20, 289 ms at 200), a Range change reset the tracker in the middle of the sound, and the harmony voices carried a DC.

### Tuner

The same YIN tracker over one wide band (27 to 1600 Hz) at about 6 kHz every 20 ms finds the period to within a frame or so; then the difference function at the full rate on a 20 ms window, at the lags around that period and around 8, 64 and more whole periods (up to 45 ms), gives it to a few thousandths of a frame. The reading glides while the note holds, jumps when it moves more than 30 cents, and holds for 300 ms after the sound stops. It passes the sound through, or mutes it. It met every round 5 target before any fix (H14), so round 5 leaves it as it is.

## 5. Controls

| Effect | Controls (in id order) |
|---|---|
| Pitch shifter | Pitch (±24 semitones), Fine (±100 cents), Spread (0 to 50 cents), Window (20 to 120 ms), Feedback (0 to 90 %), Mix |
| Pitch correct | Key, Scale, Speed, Amount, Range, Mix, the Note and Correction meters, Flex-Tune and Humanize (0 to 100 %, both 0 by default; ids 8 and 9, on the Voice page with Range and Mix) |
| Harmonizer | Key, Scale, Voice 1, Level 1, Pan 1, Voice 2, Level 2, Pan 2, Glide, Dry, Range, and the Note meter |
| Tuner | Reference, Mute, and the Note, Cents and Frequency meters |

PR 6 adds Flex-Tune and Humanize at 0, which leaves a saved pitch correct as it was set; no other control or id changes in PR 5 or PR 6, so saved projects open as they were. The Window's default moved from 50 to 30 ms (a new shifter is tighter); a saved shifter keeps its Window and now splices where the sound lines up, so every note comes out clean, at the same level and on pitch, and its delay is about half its Window, as before.

## 6. Defaults and presets

- **Pitch shifter** an octave up, Window 30 ms, Mix 100 %. Presets: Octave down (-12, Window 50: the octave down's splices are longer, and a low note's period is long), Fifth up, Wide doubler (0 semitones, Spread 24 cents), Rising stairs (+5, Feedback 60 %), Chipmunk (+19, Window 25).
- **Pitch correct**: Key C, Chromatic, Speed 20 ms, Amount 100 %, Mid, Mix 100 %. Presets: Natural (Speed 30, Flex-Tune 10, Humanize 60: a gentle pull that leaves slides and held vibrato; before PR 6, Speed 60 and Amount 80), Hard tune, A minor snap (Speed 2; 5 before PR 6, when Speed ran faster than its label), Pentatonic glide, Low voice.
- **Harmonizer** and **Tuner**: [fx.md](fx.md).

## 7. Measuring quality

fxlab's `pitch` class (`quality/round5.rs`, ids H) measures them, and the ratchet holds every number ([fx-quality.md](fx-quality.md#round-5-movement-pitch-and-voice) has the targets and the history):

- **Any note clean** (H1): 13 sines from 110 Hz to 1.8 kHz at +12, +7, -5 and -12 semitones, at the default Window: each tone's energy off the target pitch (≤ -30 dB), its mean pitch (within 5 cents) and its level swing (≤ 3 dB).
- **Chords** (H2): A3, C#4 and E4 saws an octave up, the energy off the target partials (≤ -15 dB).
- **Transients** (H3): an impulse at +12, its copies over -12 dB and their spread (reported).
- **Delay** (H4): a click train's energy centroid against the input's, at the default Window (≤ 15 ms, reported 0).
- **Highs** (H5): pink noise at +1 semitone, the 12 to 16 kHz band against the input's tilt (within 0.5 dB).
- **Clicks on every control:** round 3's M13 (≤ -60 dB); Feedback is left out, since stairs move the sound past 6 kHz by design.
- Unit tests in `pitch.rs`: 56, 112, 440 and 1760 Hz an octave up and down come out at their level (within 0.02) with everything else under 1e-3; the mean delay at +12, +7, -5 and -12 under 15 to 22 ms, and none reported; the twelve-tap read passes the lows exactly at every fraction.

Pitch correct and the harmonizer (H6 to H13):

- **Accuracy** (H6): saws across each Range's span, Key C major at Speed 0, within 3 cents of the nearest note on average and wobbling at most 2; each harmony voice within 3 cents of its interval.
- **Vibrato** (H7): a ±40-cent vibrato at 5.5 Hz on A4, at most 5 cents left at Speed 0, at least 90 % kept at Speed 100.
- **Settling** (H8): a saw 30 cents sharp from silence, within 5 cents for good in at most 1.8 × Speed + 25 ms after the latency.
- **Smoothness** (H9): the energy off the harmonics, in tune (within 3 dB of the input's own), correcting 45 cents (≤ -50 dB), and each harmony voice (≤ -40 dB).
- **Formants** (H10): an "AH" at 130 Hz, F1 and F2 within one harmonic after an octave up or down or a 5th up.
- **Noise and notes** (H11): white noise within 0.5 dB; the Note meter right on a sine, saw, square and missing fundamental at A2, A3 and A4.
- **Latency** (H12): pink's delay within a frame of the reported latency.
- **Clicks** (H13): every control at most -60 dB, Flex-Tune and Humanize among them.
- **The pitch meter** (`tone_cents`, PR 6): the pitch is the phase's slope in a soft band around the note it should be (full within 15 % of it, a cosine down to nothing at 30 %), smoothed over 2.2 ms, its first and last tenth left out. The meter before it cut the band off hard, and its ringing read 2 to 3 cents of wobble on a plain saw at 70 to 100 Hz, more than the effect's own; it reads 0.02 to 0.4 now. The rotary's Doppler keeps the old meter.
- Unit tests in `vocal.rs`: a voice at ratio 1 is the input (within 1e-5); the period is refined to a hundredth of a frame (82.4, 220, 451.6 and 1046.5 Hz); Flex-Tune leaves notes sung far off alone; Humanize slows a held note; a harmony voice carries no DC (its mean under 0.5 % of its RMS; 5 % without the blocker).

## 8. Cost

The instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz, at the defaults:

| Case | Before round 5's fixes | Now |
|---|---|---|
| Pitch shifter | 59.0k | 72.0k |
| Pitch correct | 60.1k | 74.4k |
| Harmonizer | 99.3k | 99.1k |
| Tuner | 32.2k | 32.2k |

PR 5's pitch shifter costs 22 % more than the two-head shifter it replaces: about half of it is the twelve-tap read (the old read was two linear interpolations a voice), the rest the search, about 45 times a second a voice. The first version cost 46 % more: its sums were vectorised two lanes wide, pairing each sample's two products, until the lanes were summed in order (`sum4`), and the read took two float conversions where one does.

PR 6's pitch correct costs 24 % more: the twelve-tap reads (stereo, two a frame, where the grains overlap), the period found again at the full rate every look, and the looks lined up with the grains. The harmonizer's two mono voices read one channel each, and it costs what it did. The first version cost 60 % and 45 % more, until a grain was laid 128 frames at a time, four frames at a time through the taps (a window of 15 frames as a fixed-size array, so the compiler vectorises it), the low-passed copy was written twice instead of copied for each refinement, each ratio was worked out once a change, and the read hint was cast through 32 bits (a float to a 64-bit integer is a library call on ARM).

## 9. Pitfalls and how the design avoids them

- **Splices at fixed points** (the shifter before PR 5): some notes nearly cancelled (68.7 dB under the rest at worst), the pitch slid up to 114 cents and the level swung 12 dB. The splice is searched for.
- **A crossfade of unrelated sounds dips:** on chords the best match still differs. The crossfade's gain comes from the match.
- **A search on whole frames:** a high note's period isn't a whole number of frames, and the error is heard as a small buzz at the splice rate. The best frame is refined between frames by a parabola.
- **A coarse search that misses the peak:** the decimated copy finds the region, the full rate the frame. A copy at an eighth of the rate keeps everything under about 2.7 kHz, which is where the match is decided.
- **A linear read** (before PR 5): the highs 2.3 dB dull at +1 semitone, and an image-filled octave down. Twelve taps of windowed sinc, and a low-pass going down.
- **An eight-tap sinc:** its images were 62 dB down, and the click measure heard them on a Pitch sweep (-58.7 dB). Twelve taps put them at 76 dB.
- **An old read that runs into the write** while it fades out, shifting up by a lot: the splice comes early by the fade's travel, and the fade is shorter at big shifts.
- **A long default window** (50 ms before PR 5): a 25 ms slapback against the dry. 30 ms, with aligned splices, lags about 11 ms.
- **A period found again on the full band** (a step on the way in PR 6): the difference function's dip is a V whose bottom moves with the overtones, and it read up to 3 cents off now and then. The ratio jittered with it, and the jitter was heard as roughness (a note in tune came out 4.6 to 6 dB rougher than it went in). The period is found on the low-passed copy, where the dip is round.
- **A straight line between frames:** each grain loses a different share of its highs, with its fraction, and the change from grain to grain is heard as roughness. The twelve-tap sinc reads every grain alike; on a whole frame it reads the frame, since the sinc's ripple took 0.6 dB off noise.
- **An ease that switches:** near ratio 1 the grains ease back to their marks; switched on and off at one threshold, it made 9-cent pulses on a 700 Hz note. It fades in between 0.1 % and 0.03 % of ratio 1, and moves by at most 0.1 % of a period a grain, which a vibrato doesn't hear.
- **A smoothstep window:** its overtone floor put the harmonizer's Voice and Pan clicks at -59.5 dB. A smootherstep is flat in curve as well as slope at its ends.
- **Uneven overlap:** shifting up by less than an octave, the grains overlap more on one side of a period than the other, and the weighted sum leans: a DC 28 dB under the voice. A blocker at 5 Hz on each voice.
- **A look stamped when it was made:** an unvoiced look stamped at the moment it was made held a note's start back until the next voiced look reached the grains (21.9 ms at Speed 0). It is stamped at the middle of what it heard, and the next voiced note fills it in.
- **A meter that rings:** the brick-wall band read its own ringing as the effect's wobble. Soft band edges.

## 10. History and next

- F2: the pitch shifter (`pitch.rs`); the vocal effects: pitch correct and the harmonizer (`vocal.rs`) and the tuner (`tuner.rs`).
- FX quality round 5 (17 Nov): research, baseline and plan (mpfree-claude #211, plan-5.md); the measuring PR (#510): 53 of 82 pitch targets met.
- PR 5 (18 Nov): the pitch shifter rebuilt as WSOLA (aligned splices searched coarse, fine and between frames; a match-gained smoothstep fade), a twelve-tap sinc read, a low-pass going down, and a 30 ms default Window: every pitch shifter target met.
- PR 6 (18 Nov): pitch correct and the harmonizer: the period found again on the low-passed copy, looks stamped at what they heard and read where the grains are, marks and grains between frames through the twelve-tap sinc, smootherstep windows, a gentle ease near ratio 1, Speed as a one-pole of Speed ms, a dip on a Range change, a DC blocker on each harmony voice; Flex-Tune and Humanize (plan-5 decision 12.3), and the Natural preset made with them. Every pitch target met (84 of 84). The Range change dips instead of crossfading two trackers as the plan said: a dip of 5 ms either side costs nothing while it isn't moving, and is under -85 dB on the click measure.
- Next: the talk box and the vocoder (round 5's PRs 7 and 8, plan-5 sections 10 and 11) are held for a later session; their design page will be fx-voice.md. Not targets: the shifter's formants (it has no pitch to keep them by; pitch correct and the harmonizer do), and its transients (H3, reported: a hit near a splice is heard twice, 15 ms apart at most at the default Window).

## 11. Sources

- research-pitch.md (jferments/mpfree-claude `fx/quality/`) and its sources: Valhalla DSP on the H949 and de-glitching; Eventide's H949 pages; Sound On Sound's review of Antares AutoTune 2026; Boss's TU-3 specifications; W. Verhelst and M. Roelands, "An overlap-add technique based on waveform similarity (WSOLA)", ICASSP 1993; E. Moulines and F. Charpentier, PSOLA, Speech Communication 1990; A. de Cheveigné and H. Kawahara, "YIN", JASA 2002.
