# Plucked strings and struck bars: Karplus-Strong strings and modal mallets

**Status:** research, 2026-11-19 (synth library wave 3, polishing Pluck). Deepens the string parts of [Keys](keys.md) and the velocity and level parts of [What every type shares](synth-topics.md), which set the shared rows (L2, L5, L6, L11) Pluck is measured against. The engine is [Pluck](../design/synths.md#pluck-mpfree-synth-pluck); its targets, before and after, are in [Synth quality](../design/synth-quality.md#wave-3-targets-pluck-stage-7).

It answers the same eight questions as the other synth research pages. Where a point comes from a source it is named; where it is our own reading, arithmetic or a measurement from the engine, it says so. The owner chose pure synthesis: no recorded instruments.

## 1. What makes a great one

- **The pluck's place shapes the tone.** A string plucked at a fraction β of its length has no harmonics with a node there: plucked at the middle, no even harmonics; at a fifth, no 5th, 10th, 15th. In a digital string that is a comb, `1 - z^(-βN)` for a loop N frames long (Jaffe and Smith 1983; Smith, *Physical Audio Signal Processing*). The notches have to land on the harmonics the string plays, or the pluck's place barely shows: MPFree's first comb worked inside the noise burst's one period, so plucked at the middle the even harmonics were only 7 dB down (our measurement, U1).
- **Higher harmonics die sooner, higher notes ring shorter.** The loop's low-pass takes a little more off each harmonic on every trip, so the tone darkens as it rings, and a high note, making more trips a second, rings shorter (Karplus and Strong 1983; Jaffe and Smith 1983). Pluck's Decay follows a −0.5 law against frequency (K3).
- **A bar's overtones are not harmonics.** A uniform bar free at both ends has modes at 1 : 2.76 : 5.40 : 8.93 (a glockenspiel's steel bars are close to this); marimba bars are cut away underneath to tune the second mode two octaves up and the third about three octaves and a major third (1 : 4 : 10), and xylophone bars the second mode a twelfth up (1 : 3) (Fletcher and Rossing, *The Physics of Musical Instruments*, chapter 19). A chime (a tube) is heard at a strike note about an octave under its 4th mode, whose 4th, 5th and 6th modes sit near 2 : 3 : 4 (same source); Pluck's Chimes table puts its played note there.
- **Velocity changes the tone, not just the level.** A harder pick or mallet is brighter. For a mallet the contact gets shorter as it lands harder: under Hertz's law for an elastic contact the time goes as the speed to the −1/5 power, and felt hardens as it is squeezed, which widens the spread (our reading of Hertz contact theory; the time-domain bar and mallet model of Chaigne and Doutaut 1997 is the reference). The research proposes a square-law level curve over 30 to 40 dB (Dannenberg 2006, as [Csound's `ampmidid`](https://csound.com/manual/opcodes/ampmidid/)).
- **Every pluck is a little different, but not 5 dB different.** A noise burst gives each harmonic a random level. That's the life of a Karplus-Strong string, but at the low harmonics, where most of the loudness is, one pluck came out up to 5.7 dB louder than the next (our measurement). Setting the first few harmonics to their average level and leaving their phases and the upper harmonics random keeps the life and takes out most of the lottery (1 to 2.5 dB after, our measurement).

## 2. How musicians use it

- **On pads, one sound per kit or track:** a marimba or kalimba line, a harp arpeggio, a plucked bass. Pads send a narrower spread of velocities than keys, so the dynamic range per preset matters (see [Keys](keys.md#7-playing-it-on-the-mpc)).
- **Chords:** strummed or struck at once. Eight bars struck together add their attacks; they mustn't clip (L2).
- **Across the keys:** a melody climbing two octaves should stay as loud. A string whose high notes ring shorter sounds quieter up high unless the engine evens it out; one whose low notes have little energy where the ear is sensitive sounds quieter down low (L11).
- **Players expect the classic names:** nylon and steel guitars, harp, koto, oud, banjo, harpsichord, pizzicato, an upright or muted bass; marimba, vibes, glockenspiel, xylophone, celesta, kalimba, steel drum, bells, chimes, music box. Generic names only: no product or trademark names.

## 3. Algorithms

- **Strings: Karplus-Strong with Jaffe and Smith's extensions.** A delay line one period long, a one-pole loop low-pass for the damping, a first-order allpass for the fraction of a frame the length needs, and the tuning takes the low-pass's phase delay off the line (Jaffe and Smith 1983). Välimäki, Huopaniemi, Karjalainen and Jánosy (1996) give the commuted and fractional-delay versions used in real time.
- **The pick position as an output comb.** Rather than combing the burst (which only works within its one period), Pluck combs the string's output: the string now, less the string as it was βN frames ago, read from the string's own delay line, linear between frames. The loop is linear, so this is the same comb as on the excitation, with exact notches at every harmonic (our design; U1 measures −50 dB at the middle). A string that was silent has nothing to subtract βN frames ago, so the tap reads silence until the pluck reaches it, and the note starts from zero (no click, B4).
- **Bars: a modal bank.** Each mode a two-pole resonator at its ratio of the note, its level and its own decay, struck by a half-sine pulse as long as the mallet stays on the bar; the pulse's spectrum weighs the modes, so a shorter, harder hit reaches higher ones. The bank is a shared part (`modal.rs`) that Drums builds toms, congas and bells on.
- **Even keys from a loudness model.** The engine works out, from the knobs, how loud each key will be heard: every harmonic or mode's K-weighted power and decay, summed into 400 ms blocks every 100 ms over a held note's first 1.5 s and gated as [ITU-R BS.1770](https://www.itu.int/rec/R-REC-BS.1770) gates them (an absolute gate, here 40 dB under the loudest block, then a relative gate 10 dB under the mean). Then it plays each key at the level that makes it as loud as middle C. A first try with a fixed listening span (the first 0.15 s of a string) over-lifted short, bright high notes by 3 to 5 dB, because the gate hears a short note over its first block, not averaged with the silence after it (our measurement).
- **What a sitar needs.** The sitar's and tanpura's buzz comes from a broad, curved bridge the string wraps against as it swings: the effective length changes through each period, which feeds energy up into the high harmonics (Raman 1921 described the bridge's effect; van Walstijn, Bridges and Mehes 2016 built a real-time tanpura model on a contact with the bridge). We tried two cheap versions in the loop: a one-sided clip (it soaked up energy and pushed the loop off centre until it no longer touched, with little added brightness), and a loop that shortens by how far the string swings past the bridge (erratic: more Buzz did not mean more buzz). Neither is in the engine; a contact model like the tanpura paper's is the next step.

## 4. What to measure

- **U1, the comb:** a plain string plucked 16 times at C3, C4 and C5, each pluck's power averaged: at Position 50 the even harmonics against the odd, at Position 20 the 5th against the 4th and 6th. Under −30 dB (our target: deep enough that the place clearly shows; the ideal comb's notch is total).
- **The shared rows:** L2 (no 8-note chord clips), L5 (brightness from velocity 20 to 127 at least 0.1 octave), L6 (a square law; the range shown, the research's 30 to 40 dB from velocity 1 to 127 being 18 to 24 dB from 20 to 127), L11 (single notes from C1 to C7 within 3 dB), L12, and K3 for Decay's label and law. Definitions in [Synth quality](../design/synth-quality.md#the-report).
- **Per-pluck variation:** the same key plucked 12 times, its loudness range. Not a row (the lab plays each key once), but measured while building: 1 to 2.5 dB.

## 5. Knobs and defaults

- **Model** (String or Mallet). **Decay** (time to fall 60 dB at middle C; 50 ms to 30 s), **Release** (how fast a let-go note damps; 1.5 s by default so pad taps ring).
- **String:** **Brightness** (the loop filter), **Pick** (the burst's brightness), **Position** (2 to 50 % of the string).
- **Mallet:** **Material** (nine tables), **Hardness** (the pulse length), **Strike** (where on the bar), **Stretch** (spreads the mode ratios), **High damp** (shortens the upper modes).
- **Voice:** **Vel > tone** (how much darker a soft note is: up to 4 octaves off the pick's cutoff, or a pulse up to 8 times longer), **Velocity** (how much quieter the softest note is than the hardest, 0 to 48 dB, 30 by default: the research's range, at the low end because a pad's soft hits are hard to land), Transpose, Fine, Voices, Bend range, Level, Pan, Spread.

## 6. The presets players expect

Strings: Nylon, Steel String, Harp, Koto, Oud, Banjo, Harpsichord (little dynamics on purpose, as the instrument has: Velocity 12 dB), Pizzicato, Muted Bass, Upright Bass. Mallets: Marimba, Vibes, Glockenspiel, Xylophone, Celesta, Kalimba, Steel Drum, Bell, Glass, Chimes, Music Box. Sitar and tanpura wait for the bridge (section 3). Each preset's phrase sits within 1 dB of −25 LUFS, where the other engines' plucked sounds are (our measurement), with its 8-note chord under 0 dBFS.

## 7. Playing it on the MPC

- A re-struck ringing note adds to itself in its own voice, as a real string or bar does; past the voice limit the oldest note fades over 3 ms.
- The note-on work grew: the low-harmonic fix (about 16k frames of arithmetic for a low note, much less for a high one) and each key's level worked out once and kept until a knob moves. The string's per-frame cost rose about 11 % for the comb (instruction counts, workstation). Both want a device round before the voice count's default moves.

## 8. What goes wrong

- **A comb that doesn't notch:** combing only the burst leaves the notches partial (section 1).
- **Clicks at the start:** a comb that reads the burst before the string has played it puts a step on the first frame (B4 caught it on the bass presets: −22 dBFS, now −57).
- **A key-evening model that is too clever or too simple:** a fixed listening span tilts short, bright presets; a model that follows the loudness measure's own blocks and gates doesn't (section 3).
- **Velocity that only darkens what is already dark:** a soft pulse for a bar whose upper modes die in a few tenths of a second hardly changes the sound over the first 370 ms. Marimba, vibes, xylophone and kalimba needed harder mallets and less High damp for velocity to show (L5).

## What MPFree has today

Pluck as described: the output comb, the low-harmonic fix, the loudness model, Velocity, nine materials and 22 presets (synth library wave 3, 19 Nov). No buzzing bridge yet.

## Sources

- K. Karplus and A. Strong, "Digital Synthesis of Plucked-String and Drum Timbres", *Computer Music Journal* 7(2), 1983.
- D. Jaffe and J. O. Smith, "Extensions of the Karplus-Strong Plucked-String Algorithm", *Computer Music Journal* 7(2), 1983.
- J. O. Smith, [*Physical Audio Signal Processing*](https://ccrma.stanford.edu/~jos/pasp/), CCRMA, Stanford: the extended Karplus-Strong string and its pick-position comb.
- V. Välimäki, J. Huopaniemi, M. Karjalainen and Z. Jánosy, "Physical Modeling of Plucked String Instruments with Application to Real-Time Sound Synthesis", *Journal of the Audio Engineering Society* 44(5), 1996.
- N. H. Fletcher and T. D. Rossing, *The Physics of Musical Instruments*, Springer, chapter 19 (bars, marimbas, xylophones, glockenspiels, chimes).
- A. Chaigne and V. Doutaut, "Numerical simulations of xylophones. I. Time-domain modeling of the vibrating bars", *Journal of the Acoustical Society of America* 101(1), 1997.
- R. Dannenberg, "The Interpretation of MIDI Velocity", ICMC 2006.
- [ITU-R BS.1770](https://www.itu.int/rec/R-REC-BS.1770), algorithms to measure audio programme loudness.
- C. V. Raman, "On some Indian stringed instruments", *Proceedings of the Indian Association for the Cultivation of Science* 7, 1921.
- M. van Walstijn, J. Bridges and S. Mehes, "A Real-Time Synthesis Oriented Tanpura Model", DAFx 2016.
