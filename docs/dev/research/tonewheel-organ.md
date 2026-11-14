# Tonewheel organs and rotary speakers: what the research says

**Status:** research, 2026-11-14. Feeds the [Drawbar engine](../design/synths.md#drawbar-mpfree-synth-drawbar). Nothing here has run on the MPC.

The owner asked for instruments researched from papers and journals before they were built. This page collects what the measurements, papers and service documentation say about the electromechanical tonewheel organ and the rotating speaker it is usually played through, and what a real-time engine on the MPC One takes from them. Where a point comes from a source it is linked; where it is our own reading it says so.

## The tone generator

- **91 wheels on one motor.** A synchronous motor turns the driving shaft at 1200 rpm (20 turns a second). Each of the 12 notes has its own pair of gears, repeated in every octave, and a wheel's pitch is 20 × the gear ratio × its number of teeth: 2, 4, 8 ... 128 teeth for the first seven octaves, then seven wheels of 192 teeth at the top, because 256-tooth wheels could not be cut ([Electric Druid](https://electricdruid.net/?p=316); [ToneWheelSpec](https://goodeveca.net/RotorOrgan/ToneWheelSpec.html)). The gear ratios are 85/104 (C), 71/82, 67/73, 105/108, 103/100, 84/77, 74/64, 98/80, 96/74, 88/64 (A, so A 440 Hz is exact), 67/46 and 108/70 (B). The top seven wheels take the gears of the note a fourth above.
- **Nearly equal temperament.** The gearing is at most about 0.7 cents off equal temperament (G#, 0.71 cents flat by our count from the ratios) in the first seven octaves, and up to about 1.9 cents sharp at the top ([Electric Druid](https://electricdruid.net/?p=316); [Werner & Abel, Applied Sciences 2016](https://www.mdpi.com/2076-3417/6/7/185)). The wheels run from about 32.7 Hz to 5924.6 Hz.
- **Tempered harmonics.** The drawbars take the nearest wheel, so the fifth-based harmonics are about 2 cents flat of pure and the third-based (the 1 3/5' drawbar) about 13.7 cents sharp (Werner & Abel 2016).
- **Waveform.** Each wheel aims at a sine. The lowest octave (pedals only) is cut closer to a square, with the 3rd and 5th harmonics present; Werner and Abel model it as a sum at 1×, 3× and 5×.
- **Crosstalk.** Two wheels share each compartment on the same driven gear, four octaves apart, and leak into each other magnetically; filtering added in 1963 reduced leakage and hum. Players now count it as part of the sound ([Electric Druid](https://electricdruid.net/?p=316); [Wikipedia, Hammond organ](https://en.wikipedia.org/wiki/B3_organ)). setBfree, an open-source organ, defaults its compartment, terminal-strip and wiring crosstalk to 0.01 each (-40 dB) ([setBfree man page](https://mankier.com/1/setBfree)).
- **Constant phase.** The wheels turn all the time, so a key switches on wheels that are already running: no two notes start at the same phase, and a wheel shared by two keys (one key's 4' and the next octave's 8') adds up in one wheel rather than beating. This is our reading of the mechanism.

## Drawbars and foldback

- Nine drawbars per manual: 16', 5 1/3', 8', 4', 2 2/3', 2', 1 3/5', 1 1/3' and 1', the sub-octave, the third harmonic an octave down, and harmonics 1, 2, 3, 4, 5, 6 and 8, at -12, +7, 0, +12, +19, +24, +28, +31 and +36 semitones ([Sound On Sound](https://www.soundonsound.com/techniques/synthesizing-tonewheel-organs-part-1); Werner & Abel 2016).
- Each drawbar has 9 positions, 0 to 8, about 3 dB apart (Werner & Abel 2016).
- **Foldback.** The manuals use wheels 13 to 91 (65.4 Hz to 5924.6 Hz). The 16' drawbar in the lowest manual octave folds back an octave (wheels 13 to 24, doubling the 8'), and harmonics that would go past wheel 91 fold back an octave at the top, so the top keys get their highest drawbars an octave low ([Electric Druid](https://electricdruid.net/?p=316)).

## Key click

Each key closes nine contacts on nine busbars, carrying audio. They close at slightly different moments and bounce, so the start of a note is a burst of very fast random switching of running sine waves: the click ([Electric Druid](https://electricdruid.net/?p=316)). It was first treated as a defect and filtered; players adopted it ([Wikipedia](https://en.wikipedia.org/wiki/B3_organ)). Pekonen, Pihlajamäki and Välimäki imitate it cheaply with an extra 6th harmonic of 70 ms decay ([DAFx 2011](https://dafx.de/paper-archive/2011/Papers/49_e.pdf)); setBfree sets click lengths of 0.125 to 0.625 ms and a release click at half the level of the attack's.

## Percussion

- A decaying 2nd or 3rd harmonic added at each note's start, upper manual only, normal or soft volume, fast or slow decay ([Wikipedia](https://en.wikipedia.org/wiki/B3_organ)).
- **Single trigger:** it fires only when a key goes down with no other key held, and re-arms when all are released, so legato playing has no percussion. It is polyphonic: a chord struck together all gets it.
- It takes the 1' drawbar's contact as its trigger, so the 1' drawbar goes silent with percussion on ([Electric Druid](https://electricdruid.net/?p=316)); normal volume also lowers the sustained drawbars slightly ([Hammond B-3 notes](https://fubsypoly-musicstudio.pages.dev/hammond-b3/vol-3), a secondary source).
- setBfree's defaults: fast decay 1 s, slow 4 s, soft 0.5012 of normal (-6 dB).

## Scanner vibrato and chorus

Werner, Dunkel and Germain modelled the circuit with wave digital filters ([DAFx 2016](https://dafx.de/paper-archive/2016/dafxpapers/38-DAFx-16_paper_54-PN.pdf)):

- An LC ladder (a lumped delay line) of 19 taps, about 45 µs a section, 0.85 ms at the last tap, with a passband to about 7 kHz and ripples of around 6 dB.
- A capacitive scanner turning at about 6 to 7 Hz sweeps back and forth across the taps. V1 scans taps 1 to 9, V2 up to 13, V3 up to 19 (so about 0.40, 0.58 and 0.85 ms). Moving between neighbouring taps mixes two delays, which puts comb-like notches in the spectrum.
- Chorus (C1 to C3) uses the same depths with the dry signal mixed back in.
- Other estimates: 412 rpm, about 6.9 Hz (the B-3 notes above); setBfree uses 7.25 Hz.

## Overdrive

The organ's preamp and the rotary speaker's valve amplifier distort when pushed; players drive them on purpose. Pekonen et al. leave it out; setBfree models a biased valve stage. Our reading: an asymmetric soft curve (even harmonics), before the speaker, is the cheap version.

## The rotary speaker

- **Two rotors, split at 800 Hz:** a horn (one mouth and a dummy for balance) above the crossover and a rotating baffle over a downward-facing woofer below it ([Wikipedia, Leslie speaker](https://en.wikipedia.org/wiki/Leslie_speaker); [Sound On Sound](https://www.soundonsound.com/techniques/synthesizing-rest-hammond-organ-part-1)). Pekonen et al. split with 4th-order Butterworth filters.
- **Speeds:** horn about 40 to 50 rpm slow (chorale) and 400 fast (tremolo); drum 40 and 340 (Wikipedia). setBfree: horn 40.32 and 423.36 rpm, drum 36 and 357.3 rpm.
- **Inertia:** the horn changes speed in well under a second, the drum over several seconds. setBfree's time constants: horn 0.161 s up and 0.321 s down, drum 4.127 s up and 1.371 s down. The 122 model lags between the switch and the change (Wikipedia).
- **Doppler and AM:** a source on a circle of radius r turning at ω shifts frequency by about r·ω/c, done with a modulated delay line; the horn's level and tone also depend on where it faces ([Smith, Serafin, Abel & Berners, DAFx 2002](https://ccrma.stanford.edu/%7Ejos/doppler/dafx02.pdf)). Measured horn responses vary with angle mostly in level and treble. Early cabinet reflections help, as extra delayed sources. setBfree's radii: horn 19.2 cm, drum 22 cm; its horn leaks 15 % of its sound unmodulated.
- Pekonen et al. use spectral delay filters (chains of modulated all-passes) instead of delay lines, with slow 2 Hz and fast 6 Hz and an AM between 1 and 0.9. [Herrera, Hanson and Abel (AES 2009)](https://ccrma.stanford.edu/papers/view/2568) use time-varying FIR filters from impulse responses measured around the cabinet.

## What the Drawbar engine takes from this

| Finding | In Drawbar |
|---|---|
| 91 wheels from one motor through 12 gear ratios | A bank of 91 free-running wheels; **Wheels** Gears puts the gear ratios' offsets from equal temperament on top of the project's tuning, Exact plays the tuning as it is |
| Wheels already turning, shared between keys | Keys only switch wheel levels; a wheel used by two keys sums once |
| Lowest octave nearly square | Wheels 1 to 12 sum 1×, 3× and 5× (at 1, 1/3, 1/5) |
| 9 drawbars, 3 dB a step | Nine stepped drawbars 0 to 8, -3 dB a step |
| Foldback at both ends | 16' folds up in the lowest manual octave; wheels past 91 fold down |
| Crosstalk four octaves apart | **Leakage**: each wheel sounds its compartment partner (48 wheels away) at up to -26 dB (default -40 dB) |
| Nine contacts closing unevenly and bouncing | **Click**: each drawbar's contact closes at its own random moment, with a bounce, switching running wheels in well under a millisecond |
| Single-trigger polyphonic percussion, 2nd or 3rd, 1 s or 4 s, soft -6 dB, takes the 1' | The same, with normal lowering the drawbars by 3 dB (our reading of "slightly") |
| Scanner: 19-tap delay, 0.40 / 0.58 / 0.85 ms, about 7 Hz, mixing neighbouring taps | A delay line read at the taps' delays, the scanner crossfading between neighbouring taps, a 7 kHz low-pass; chorus mixes the dry signal back |
| Preamp and speaker amp overdrive | **Drive**: a smooth, biased curve (u/√(1+u²)) before the speaker, for even harmonics without the aliasing a sharp corner gives |
| Rotary: 800 Hz split, horn and drum at their speeds and inertia, Doppler, AM, reflections | A 4th-order split; each rotor a modulated delay per microphone with its level following its angle, the horn's treble falling from 9 kHz facing the microphone to 3 kHz facing away, one cabinet reflection, horn leakage, speeds and time constants as above; **Speed** Stop, Slow, Fast |

Left for later (our reading): the wheels' own small level wobble from shaft eccentricity (Pekonen et al. use 5 Hz), terminal-strip and wiring crosstalk, the per-wheel level taper of the generator's wiring, the lower manual and pedals as separate parts, the ladder's per-tap ripples, the expression pedal's loudness compensation, and a measured-response rotary (Herrera et al.).

## Open source to learn from

| Project | What | License | Notes |
|---|---|---|---|
| [setBfree](https://github.com/pantherb/setBfree) | Tonewheel organ with crosstalk, click, percussion, scanner, overdrive and a rotary (whirl) | GPL-2.0 | The most complete open model; its documented defaults are cited above. Studied, not copied. |
| [tuneBfree](https://github.com/narenratan/tuneBfree) | setBfree with microtuning | GPL-2.0 | |

Like the [plugin pack](../design/plugins.md), a GPL organ would ship only as a separate plugin, on the owner's go.
