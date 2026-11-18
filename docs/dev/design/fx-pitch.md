<!--
covers: rust/mpfree-fx/src/pitch.rs rust/mpfree-fx/src/vocal.rs rust/mpfree-fx/src/tuner.rs
checked: 2026-11-18 b4fe3b81
-->
# Pitch: pitch shifter, pitch correct, harmonizer and tuner

**Status:** built (F2 and the vocal effects); FX quality round 5 is under way. Its research, targets and measuring are done (53 of 82 pitch targets met before any fix, at 44.1 kHz). PR 5 (18 Nov) rebuilds the pitch shifter as a WSOLA shifter with aligned splices, a twelve-tap sinc read and a 30 ms default Window: every pitch shifter target met. Pitch correct and the harmonizer come next (PR 6). Below, "before PR 5" describes what was there before.

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

Both rest on a YIN tracker (de Cheveigné and Kawahara 2002) every 5 ms on the input low-passed and taken down to 6, 8 or 12 kHz for the Low, Mid and High Range, and on a PSOLA shifter: marks a period apart on the waveform's peaks, grains of two periods laid down at the new period, `sin²` windows that sum to one, the formants kept. Pitch correct moves the voice to the nearest note of the key and scale at Speed (a one-pole in cents), by Amount; the harmonizer adds up to two voices a scale's intervals away, each with a level. Both report their latency (two and a half of the Range's longest periods): 41.7, 27.8 and 16.7 ms. [fx.md](fx.md) has every control. Round 5's PR 6 places the marks and grains between frames, centres the analysis on the grain played, re-derives Speed, crossfades a Range change and adds Flex-Tune and Humanize (plan-5 section 10).

### Tuner

The same YIN tracker over one wide band (27 to 1600 Hz) at about 6 kHz every 20 ms finds the period to within a frame or so; then the difference function at the full rate on a 20 ms window, at the lags around that period and around 8, 64 and more whole periods (up to 45 ms), gives it to a few thousandths of a frame. The reading glides while the note holds, jumps when it moves more than 30 cents, and holds for 300 ms after the sound stops. It passes the sound through, or mutes it. It met every round 5 target before any fix (H14), so round 5 leaves it as it is.

## 5. Controls

| Effect | Controls (in id order) |
|---|---|
| Pitch shifter | Pitch (±24 semitones), Fine (±100 cents), Spread (0 to 50 cents), Window (20 to 120 ms), Feedback (0 to 90 %), Mix |
| Pitch correct | Key, Scale, Speed, Amount, Range, Mix, and the Note and Correction meters |
| Harmonizer | Key, Scale, Voice 1, Level 1, Pan 1, Voice 2, Level 2, Pan 2, Glide, Dry, Range, and the Note meter |
| Tuner | Reference, Mute, and the Note, Cents and Frequency meters |

PR 5 changes no control and no id, so saved projects open as they were. The Window's default moved from 50 to 30 ms (a new shifter is tighter); a saved shifter keeps its Window and now splices where the sound lines up, so every note comes out clean, at the same level and on pitch, and its delay is about half its Window, as before.

## 6. Defaults and presets

- **Pitch shifter** an octave up, Window 30 ms, Mix 100 %. Presets: Octave down (-12, Window 50: the octave down's splices are longer, and a low note's period is long), Fifth up, Wide doubler (0 semitones, Spread 24 cents), Rising stairs (+5, Feedback 60 %), Chipmunk (+19, Window 25).
- **Pitch correct**, **Harmonizer** and **Tuner**: [fx.md](fx.md).

## 7. Measuring quality

fxlab's `pitch` class (`quality/round5.rs`, ids H) measures them, and the ratchet holds every number ([fx-quality.md](fx-quality.md#round-5-movement-pitch-and-voice) has the targets and the history):

- **Any note clean** (H1): 13 sines from 110 Hz to 1.8 kHz at +12, +7, -5 and -12 semitones, at the default Window: each tone's energy off the target pitch (≤ -30 dB), its mean pitch (within 5 cents) and its level swing (≤ 3 dB).
- **Chords** (H2): A3, C#4 and E4 saws an octave up, the energy off the target partials (≤ -15 dB).
- **Transients** (H3): an impulse at +12, its copies over -12 dB and their spread (reported).
- **Delay** (H4): a click train's energy centroid against the input's, at the default Window (≤ 15 ms, reported 0).
- **Highs** (H5): pink noise at +1 semitone, the 12 to 16 kHz band against the input's tilt (within 0.5 dB).
- **Clicks on every control:** round 3's M13 (≤ -60 dB); Feedback is left out, since stairs move the sound past 6 kHz by design.
- Unit tests in `pitch.rs`: 56, 112, 440 and 1760 Hz an octave up and down come out at their level (within 0.02) with everything else under 1e-3; the mean delay at +12, +7, -5 and -12 under 15 to 22 ms, and none reported; the twelve-tap read passes the lows exactly at every fraction.

## 8. Cost

The instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz, at the defaults:

| Case | Before round 5's fixes | Now |
|---|---|---|
| Pitch shifter | 59.0k | 72.0k |
| Pitch correct | 60.1k | 60.1k |
| Harmonizer | 99.3k | 99.3k |
| Tuner | 32.2k | 32.2k |

PR 5's pitch shifter costs 22 % more than the two-head shifter it replaces: about half of it is the twelve-tap read (the old read was two linear interpolations a voice), the rest the search, about 45 times a second a voice. The first version cost 46 % more: its sums were vectorised two lanes wide, pairing each sample's two products, until the lanes were summed in order (`sum4`), and the read took two float conversions where one does.

## 9. Pitfalls and how the design avoids them

- **Splices at fixed points** (the shifter before PR 5): some notes nearly cancelled (68.7 dB under the rest at worst), the pitch slid up to 114 cents and the level swung 12 dB. The splice is searched for.
- **A crossfade of unrelated sounds dips:** on chords the best match still differs. The crossfade's gain comes from the match.
- **A search on whole frames:** a high note's period isn't a whole number of frames, and the error is heard as a small buzz at the splice rate. The best frame is refined between frames by a parabola.
- **A coarse search that misses the peak:** the decimated copy finds the region, the full rate the frame. A copy at an eighth of the rate keeps everything under about 2.7 kHz, which is where the match is decided.
- **A linear read** (before PR 5): the highs 2.3 dB dull at +1 semitone, and an image-filled octave down. Twelve taps of windowed sinc, and a low-pass going down.
- **An eight-tap sinc:** its images were 62 dB down, and the click measure heard them on a Pitch sweep (-58.7 dB). Twelve taps put them at 76 dB.
- **An old read that runs into the write** while it fades out, shifting up by a lot: the splice comes early by the fade's travel, and the fade is shorter at big shifts.
- **A long default window** (50 ms before PR 5): a 25 ms slapback against the dry. 30 ms, with aligned splices, lags about 11 ms.

## 10. History and next

- F2: the pitch shifter (`pitch.rs`); the vocal effects: pitch correct and the harmonizer (`vocal.rs`) and the tuner (`tuner.rs`).
- FX quality round 5 (17 Nov): research, baseline and plan (mpfree-claude #211, plan-5.md); the measuring PR (#510): 53 of 82 pitch targets met.
- PR 5 (18 Nov): the pitch shifter rebuilt as WSOLA (aligned splices searched coarse, fine and between frames; a match-gained smoothstep fade), a twelve-tap sinc read, a low-pass going down, and a 30 ms default Window: every pitch shifter target met.
- Next: PR 6, pitch correct and the harmonizer (H6 to H13, with Flex-Tune and Humanize). Not targets: the shifter's formants (it has no pitch to keep them by; pitch correct and the harmonizer do), and its transients (H3, reported: a hit near a splice is heard twice, 15 ms apart at most at the default Window).

## 11. Sources

- research-pitch.md (jferments/mpfree-claude `fx/quality/`) and its sources: Valhalla DSP on the H949 and de-glitching; Eventide's H949 pages; Sound On Sound's review of Antares AutoTune 2026; Boss's TU-3 specifications; W. Verhelst and M. Roelands, "An overlap-add technique based on waveform similarity (WSOLA)", ICASSP 1993; E. Moulines and F. Charpentier, PSOLA, Speech Communication 1990; A. de Cheveigné and H. Kawahara, "YIN", JASA 2002.
