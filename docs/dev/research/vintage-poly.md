# Vintage analog polysynths: what the research says

**Status:** research, 2026-11-14. Feeds the [Analog engine](../design/synths.md). Nothing here has run on the MPC.

The owner asked for instruments researched from papers and journals before they were built. The late-1970s and early-1980s analog polysynths share one plan: a voice card per note, each with one or two oscillators, a resonant filter, two envelopes and an amplifier, under a microprocessor that hands notes to the cards, with a chorus on some. Their sound is studied from datasheets (the Curtis chips most of them used), virtual-analog papers, a BBD paper that took one of them as its example, emulator source code and makers' own notes. This page collects what those say and what a real-time engine on the MPC One takes from them. Where a point comes from a source it is linked; where it is our own reading or arithmetic it says so. MPFree's [Poly](../design/synths.md#poly-mpfree-synth) already covers clean subtractive synthesis; this page is about what makes the old instruments sound old.

## Voices, and why no two are alike

- **Every voice is its own circuit, and they differ.** A maker of current analog polys tells owners that "+/- 5-10 cents variation per voice is expected given the density of analog components in the signal path", that the same spread "can also affect the frequency that each filter operates at", and that voices can differ in brightness and loudness; warmth makes oscillators and filters "very susceptible to pitch variation" ([Sequential, "Analog Synths and Expected Variations"](https://support.sequential.com/hc/en-gb/articles/17013629319698-Analog-Synths-and-Expected-Variations)).
- **Drift is slow and different in each oscillator.** No measurements were found. Practitioners' estimates: well under a cent during most notes; on a Prophet-600 about 10 cents over 30 minutes, "varying per oscillator", heard most in sync and cross-mod patches where exact ratios matter; ±5 cents is often enough for pleasant beating and ±20 is huge. Semiconductor noise is roughly 1/f, so slow wander can be large. Emulations either pick a Gaussian offset at each note (about ±5 cents at most) or drift each oscillator slowly toward a moving random target ([KVR, "Analog Drift"](https://www.kvraudio.com/forum/viewtopic.php?p=7408514)).
- **The cards take turns.** Notes go to the cards in rotation, so a repeated chord lands on different cards each time; with the spread above, each note comes out a little different (our reading of the voice assignment the [Prophet-5 description](https://bristol.sourceforge.net/prophet5.html) gives: one computer handing keyboard notes to five identical voice circuits).

## Oscillators

- **The common chip.** The CEM3340 gives a sawtooth (2/3 of the supply), a triangle (1/3) and a pulse made by a comparator on the saw against the pulse-width voltage; a pulse-width voltage past the end turns the pulse off. Comparator delay matters at high pitch. Its own sync pins "don't work the way you might expect", so instruments built the datasheet's hard-sync circuit, with oscillator A the slave on the Prophet-5 ([Electric Druid, "CEM3340 VCO designs"](https://electricdruid.net/?p=1256)).
- **Two oscillators, mixed.** Each voice of the Prophet-5 has oscillator A (saw or square with pulse width, synced to B if asked) and oscillator B (saw, triangle or square, ±7 semitones of fine tune, playable as a low-frequency source), plus noise ([Bristol's Prophet-5 notes](https://bristol.sourceforge.net/prophet5.html)). Other instruments of the time add a square sub-oscillator an octave down or let saw and pulse sound together.
- **Band limiting.** A digital saw or pulse aliases badly; the established cheap fixes are polynomial band-limited steps (polyBLEP) and differentiated polynomial waveforms ([Välimäki, Nam, Smith and Abel, "Alias-suppressed oscillators based on differentiated polynomial waveforms", IEEE TASLP 18:4, 2010](https://mac.kaist.ac.kr/pubs/jnam-taslp2010.pdf)). Oversampling a naive oscillator "adds significantly to the computational cost" ([Lazzarini and Timoney, Computer Music Journal 34:1, 2010](https://mural.maynoothuniversity.ie/4104/1/VL_New_perspectives.pdf)).

## Poly-mod and cross-mod

- **Per-voice modulation.** On the Prophet-5, Poly-Mod takes two sources that belong to each voice, the filter envelope and oscillator B, to three destinations: oscillator A's frequency, its pulse width, and the filter's cutoff. The LFO and noise go through the wheel, globally, to either oscillator's frequency and pulse width and to the filter ([Bristol's Prophet-5 notes](https://bristol.sourceforge.net/prophet5.html)). Envelope to a synced oscillator's pitch is the classic sync sweep; oscillator B into A's pitch or the filter is audio-rate FM, harsh and clangorous, "noisy when Osc-B is square" (same source).

## The filter

- **OTA four-pole low-pass.** The CEM3320 is four transconductance stages with feedback. Its pole frequency follows an exponential law, f_p ∝ exp(−V_C/V_T), and it "supports gain values above 4, which increase the amplitude of self-oscillation"; it clips at 12 V peak to peak ([MAME's CEM3320 model, BSD-3-Clause](https://git.redump.net/mame/plain/src/devices/sound/cem3320.cpp), from the datasheet).
- **Where the distortion is.** In an OTA stage the differential input saturates, so each stage is dv/dt = ω·tanh(x − v), unlike the transistor ladder's tanh(x) − tanh(v) ([KVR, "Cheap non-linear zero-delay filters"](https://www.kvraudio.com/forum/viewtopic.php?p=4934783), mystran's OTA-ladder model). The same thread shows the cheap way to keep every stage nonlinear inside a zero-delay-feedback solve: scale each stage by tanh(d)/d from the previous sample and solve one linear system.
- **Resonance thins the bass.** Four equal stages with feedback k oscillate at k = 4, and the passband falls to 1/(1 + k): "0.2 (≈ −14 dB)" for the ladder at full resonance, "≈ −15 dB" for the Octave CAT's pair of state-variable stages ([Werner and McClellan, DAFx 2020](https://dafx2020.mdw.ac.at/Proceedings/papers/DAFx2020_paper_70.pdf)).
- **Two-pole state-variable filters.** Some polys of the time used a 12 dB/octave state-variable filter with low-, band- and high-pass outputs (the CAT's integrators are OTAs, SSM2040 transconductance 1/500000 to 1/50 ℧; same paper), softer and less resonant than the four-pole.
- **Digital models.** [Huovilainen (DAFx 2004)](https://dafx.de/paper-archive/2004/P_061.PDF) modelled a ladder with tanh in every stage at 2x oversampling, and found the feedback's extra delay must be compensated for the tuning to hold. A zero-delay-feedback solve removes that delay altogether.

## Envelopes

- **CEM3310.** The attack charges toward an asymptote of 6.5 V and stops at the 5 V peak: 1.3 times the peak, so the attack is an RC curve cut off before it flattens ([MAME's CEM3310 model, BSD-3-Clause](https://git.redump.net/mame/tree/src/devices/sound/cem3310.h), datasheet constants VZ and VP). Attack, decay and release run "from two milliseconds to twenty seconds"; sustain is 0 to 100 % of the peak ([Electronics & Music Maker on the CEM3310](https://www.muzines.co.uk/articles/electro-music-engineer-cem-3310/3647)). Decay and release are exponential: "Every (analogue) EG circuit I've looked at used an exponential 1/RC curve for all three" ([Ray Bellis, quoted on the W3C audio list](https://lists.w3.org/Archives/Public/public-audio/2013JanMar/0082.html)).

## Chorus

- **A BBD.** A bucket-brigade device delays by sampling and shifting through a chain of capacitors: total delay = stages / (2 × clock) ([Electric Druid, "What a BBD chorus really does"](https://electricdruid.net/investigations-into-what-a-bbd-chorus-unit-really-does/)), so a clock swept linearly by the LFO sweeps the delay as its reciprocal. BBDs alias, distort and hiss, so they sit between low-pass filters and often companders ([Raffel and Smith, DAFx 2010](https://dafx.de/paper-archive/2010/DAFx10/RaffelSmith_DAFx10_P42.pdf); [Huovilainen, DAFx 2005](https://dafx.de/paper-archive/2005/P_155.pdf), which also warps the delay to match a BBD).
- **One synth's chorus, measured.** [Holters and Parker (DAFx 2018)](https://dafx.de/paper-archive/2018/papers/DAFx2018_paper_12.pdf) modelled the BBD and filters of the Roland Juno-60's chorus: a fifth-order low-pass (and a first-order high-pass) on each side of the BBD, the clock driven by a triangle LFO, its lowest rate about 26 kHz. With 256 stages that is a longest delay near 4.9 ms (our arithmetic).
- **Its settings, from the community.** Two modes at about 0.5 and 0.83 Hz with a triangle and a deep sweep, and a third at about 9 to 9.75 Hz and shallow; the left and right sweeps in antiphase; the wet signal 1.62 dB under the dry ([Tylman, "Understanding the Juno-60 Chorus"](https://roberttylman.github.io/portfolio-site/tillysynth-chorus/index.html)). An open emulation uses ±2.1 ms around 3.5 ms for the slow modes and ±0.3 ms for the fast one, a 9 kHz low-pass before the delay and a 7.5 kHz fourth-order one after, and some hiss ([Juno-XD](https://github.com/chubbard/Juno-XD); its sources aren't given). Our arithmetic: 1.4 to 5.6 ms, consistent with the 4.9 ms above.

## What the Analog engine takes from this

| Finding | In Analog |
|---|---|
| Voices differ by ±5 to 10 cents, in filter frequency, brightness and level | **Age** gives each of 16 voice cards fixed offsets: both oscillators' tuning (up to σ 4 cents), cutoff (σ 6 %), envelope times (σ 8 %), pulse width and level (our reading of the spread) |
| Slow, independent drift, a few cents | At Age 100 each oscillator wanders by a few cents rms, a random walk smoothed over seconds (our reading) |
| Cards take turns | Notes go to the cards in rotation, so a repeated chord lands on different cards |
| Saw, triangle, pulse from a comparator; hard sync with A the slave; sub, noise | Two oscillators (Saw, Pulse, Saw+Pulse or Triangle, with polyBLEP), oscillator 1 syncable to 2, a square sub an octave under oscillator 1, white noise |
| Poly-Mod: filter envelope and oscillator B to A's pitch, pulse width and the filter | The same: **PMod env** and **PMod osc 2** amounts, to oscillator 1's pitch, its pulse width and the cutoff |
| OTA four-pole, tanh at each stage's input, oscillates past k = 4 | Four stages, each tanh(x − v), solved with zero-delay feedback, tanh(d)/d from the previous frame; **Resonance** up to k 4.1, so it whistles at 100 |
| Passband falls to 1/(1 + k) | Kept, half of it made up in gain, as in [Squelch](acid-bass.md) |
| Two-pole state-variable filters | A 12 dB low-, band- or high-pass whose damping grows with its own level, so resonance squashes instead of running away (our reading) |
| CEM3310: attack toward 1.3 times the peak, exponential decay and release, 2 ms to 20 s | The same curve and range |
| BBD chorus: delay = stages / (2 × clock), filtered, hissing, antiphase sweeps at 0.5, 0.83 and about 9.75 Hz | **Chorus** Slow, Fast and Vibe: a delay swept as the reciprocal of a triangle-swept clock, filtered before and after, a little hiss, left and right in antiphase, wet 1.62 dB under the dry |

Left for later (our reading): the comparator delay and the saw's reset time at high pitch, companders in the chorus, and comparing against recordings of real instruments.

## Open source to learn from

| Project | What | License | Notes |
|---|---|---|---|
| [MAME](https://git.redump.net/mame/plain/src/devices/sound/cem3320.cpp) CEM3310, CEM3320 | Chip models from the datasheets | BSD-3-Clause | Values cited above, with credit. |
| [Juno-XD](https://github.com/chubbard/Juno-XD) | A BBD chorus for a Korg SDK | None stated | Its settings are cited above; no code taken. |
