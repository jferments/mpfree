# Acid bass: what the circuit analyses say

**Status:** research, 2026-11-14. Feeds the [Squelch engine](../design/synths.md). Nothing here has run on the MPC.

The owner asked for instruments researched from papers and journals before they were built. The squelchy mono bass of acid house comes from one early-1980s bass line machine, the TB-303, and its sound has been studied mostly by circuit analysis: filter transfer functions worked out from the schematic, the service notes of the best-known modification (the Devil Fish), clone builders' notes, and a few theses and papers. This page collects what those say about each part of the circuit and what a real-time engine on the MPC One takes from them. Where a point comes from a source it is linked; where it is our own reading or arithmetic it says so.

## The oscillator

- **Sawtooth.** An exponential converter discharges a timing capacitor (C33); when it has fallen far enough, a pair of transistors wired like an SCR (Q25, Q27) snaps it back, and the cycle starts again. The short flat part at the reset is put down to the saturated transistors' storage time, so it is about the same length at any pitch ([x0xb0x VCO notes](https://ladyada.net/wiki/x0x/voltagecontrolledoscillator), a user-edited draft). Pitch comes from a 6-bit DAC (a 200k resistor network); the whole CV, as one voltage, sets the pitch.
- **Square.** There is no second oscillator: one transistor (Q8) works as a threshold detector on the sawtooth. Its gain is limited, so the transition in the middle of the cycle is slower than the reset edge: the wave is sharp at the start and end of the cycle with an S-curve in the middle, and some of the sawtooth's even harmonics come through, more so at higher pitches. The two halves are not equal, and the circuit does nothing to make them so; the duty differs from unit to unit ([KVR thread on the square wave](https://www.kvraudio.com/forum/viewtopic.php?p=3732152): posts by Gamma-UT and mystran). Measured at the output, the square is also shaped by the filter ([Kernmayer 2020](https://phaidra.ustp.at/api/object/o:4344/download), a master's thesis).

## The filter

- **A four-pole diode ladder, not three.** It is often called an 18 dB/octave three-pole filter, but SPICE runs give -24.1 and -23.9 dB/octave, and the transfer function has four poles ([Stinchcombe, "Diode ladder filters"](http://www.timstinchcombe.co.uk/synth/diode_18_24/diode.html)). Kernmayer agrees.
- **One capacitor is half the others.** Three stages use 0.33 µF and the input stage's (C18) is 0.18 µF ([x0xb0x VCF notes](https://www.ladyada.net/wiki/x0x/voltagecontrolledfilter)). Stinchcombe's transfer function for the ladder with that capacitor halved, normalised, is H(s) = -1 / (s⁴ + 6.727 s³ + 14.142 s² + 9.514 s + 1), with real poles at -3.24, -2.33, -1.04 and -0.13. The fourth pole sits well inside the band that matters and can't be dropped.
- **A nodal model gives that exactly (our derivation).** Four capacitor voltages, the input pair's current driving the first, a diode pair between each pair of neighbours and one from the last to a fixed point: v1' = 2ω(x − v1 + v2), v2' = ω(v1 − 2v2 + v3), v3' = ω(v2 − 2v3 + v4), v4' = ω(v3 − 2v4), output v4. Its characteristic polynomial, normalised, is Stinchcombe's to the third decimal, with unity gain at DC. With feedback x = input − k·v4 it starts to oscillate at k = 17, at 2^¼ (1.19) times the normalised frequency. Csound's diode ladder (after Pirkle and Zavalishin) uses the same 0 to 17 range ([Csound manual](https://csound.com/docs/manual/diode_ladder.html)). In the circuit each diode pair's current follows tanh of the voltage across it, which is where the filter's own saturation comes from.
- **Resonance thins the bass.** As resonance rises, the peak stays at about the same level and the passband falls ([x0xb0x VCF notes](https://www.ladyada.net/wiki/x0x/voltagecontrolledfilter)); in the linear model, the gain at DC is 1/(1 + k), -25 dB at k = 17.
- **A high-pass in the resonance loop.** Measuring a clone, a1k0n found the filter "works almost exactly like the theoretical model" except for a high-pass in the feedback path, plus more high-pass between the filter and the output amplifier ([a1k0n/303](https://github.com/a1k0n/303)). It is why the filter "will only self oscillate at mid and high frequencies" ([Devil Fish manual](https://www.firstpr.com.au/rwi/dfish/Devil-Fish-Manual.pdf)). Stinchcombe's full model with the coupling capacitors adds five poles near 10 Hz and a second, low resonant peak. Neither gives the loop's corner frequency.
- **Weak low end.** "In the TB-303, 32 Hz was down by 5 dB" (Devil Fish manual).
- **Range and tracking.** The filter does not follow the note: "the filter frequency was not affected by the pitch of the note" (Devil Fish manual). Its highest resonant frequency is not stated, but the Devil Fish's 5 kHz is "an octave above the typical maximum", so about 2.5 kHz.
- **Modelling the nonlinearity.** The diode ladder's stages load each other, so its distortion is less predictable than a transistor ladder's; Kernmayer builds it with Zavalishin's topology-preserving transform (zero-delay feedback). For transistor ladders, [Oyama (DAFx 2026)](https://www.dafx.de/paper-archive/2026/papers/DAFx26_paper_29.pdf) compares implementations against SPICE: putting the nonlinearity only at the input and the feedback matches the linear response but misses the harmonics, and when only some stages can be nonlinear, the early stages matter most. A cheap way to keep every stage nonlinear inside a zero-delay-feedback solve is to scale each stage by tanh(d)/d taken from the previous sample's state, which leaves one linear system a sample ([KVR, "Cheap non-linear zero-delay filters"](https://www.kvraudio.com/forum/viewtopic.php?p=6076114)).

## Envelopes and the amplifier

- **Two decay envelopes.** The volume envelope (VEG) has a sharp attack and a fixed exponential decay of about 3 to 4 seconds and drives only the VCA. The main envelope (MEG), mostly for the filter, has a sharp attack and an exponential decay set by the Decay knob, 200 ms to 2 s ([Whittle, "Unique aspects of the TB-303's sound"](https://www.firstpr.com.au/rwi/dfish/303-unique.html); Devil Fish manual).
- **Env mod can't reach zero** on the original (Whittle). Turning it up also moves the bias of the exponential converter so the cutoff falls, as if Cutoff were turned down ([x0xb0x VCF notes](https://www.ladyada.net/wiki/x0x/voltagecontrolledfilter)): more sweep starts lower.
- **Onset and end.** A note is heard 1 to 5 ms (typically 4) after its gate, with a soft attack of about 3 ms; at the end of the gate there is about 16 ms of decay, 8 ms at full volume then 8 ms falling linearly (Devil Fish manual).

## Accent

- **The envelope gets short.** An accented note's MEG decays in a fixed 200 ms, whatever the Decay knob says (Devil Fish manual; Whittle).
- **Louder, with a softer attack.** The MEG passes through the Accent knob and a 47k resistor and 0.033 µF capacitor into the VCA's control. That is "the main reason accented notes are louder", and the RC softens their attack (Whittle); its time constant is 1.55 ms (our arithmetic).
- **The accent sweep.** A diode and 47k drive one end of a 100k pot whose other end has 1 µF to ground; the wiper feeds the filter's frequency through 100k. The pot is the second half of the Resonance knob. At low resonance the filter sees mostly the MEG pulse itself (about 100/147 of it, less the capacitor's voltage): a sharp rise and fall. At high resonance it sees the capacitor, charged through the diode and 147k: a smooth swell. The capacitor does not empty between accents close together, so each peaks higher than the last, more so with more resonance (Whittle). Its charging time constant is 147 ms (our arithmetic).

## Slide

- A slid note glides from the previous note's pitch to its own, and the gate stays open from the one note into the next, so the envelopes don't start again ([Kernmayer 2020](https://phaidra.ustp.at/api/object/o:4344/download)).
- The glide is the DAC's 100k feeding a 0.22 µF capacitor, "normally 60 ms" (Devil Fish manual). That RC is 22 ms, so 60 ms is about three time constants: an exponential glide in pitch (our arithmetic). Analog switches short the capacitor out when there is no slide ([x0xb0x VCO notes](https://ladyada.net/wiki/x0x/voltagecontrolledoscillator)).
- The sequencer runs 6 ticks to a sixteenth and opens a normal gate for 3.5 of them (Kernmayer).

## What the Squelch engine takes from this

| Finding | In Squelch |
|---|---|
| Sawtooth with a sharp reset; square from a limited-gain shaper of the saw, uneven halves | A band-limited sawtooth; **Square** is the saw through a soft threshold slightly off centre, so the middle transition is an S-curve and the halves differ |
| Four-pole diode ladder, input capacitor halved, Stinchcombe's polynomial | The nodal model above, every diode pair through tanh, solved with zero-delay feedback, the stage scaling taken from the previous sample; **Resonance** 0 to 100 % is k up to 18.5, just past 17, because with a note through it the input pair's compression holds it at the edge: at 100 the harmonic nearest a mid or high cutoff rings out rather than a free whistle (our reading) |
| High-pass in the feedback; self-oscillates only at mid and high cutoffs | A one-pole high-pass inside the loop (our reading: 120 Hz) |
| Passband falls with resonance | Kept, but half of it made up in gain (our reading), so a resonant line is not 25 dB quieter |
| 32 Hz down 5 dB | A one-pole high-pass at 47 Hz on the output (our arithmetic: -5 dB at 32 Hz) |
| No keyboard tracking | **Key track** 0 by default |
| VEG fixed 3 to 4 s; MEG 200 ms to 2 s; accented MEG 200 ms | VCA decay 3.5 s (**Amp decay** can shorten it); **Decay** 0.2 to 2 s; accented notes 200 ms (**Acc decay** can change it) |
| Env mod lowers the base as it widens the sweep | **Env mod** sweeps up to 4 octaves and lowers the base by a quarter of the sweep (our reading of "as if Cutoff were turned down") |
| Accent: louder through a 1.55 ms RC; sweep through a diode and 1 µF, mixed by the Resonance knob | The same: the accented MEG into the VCA through a 1.55 ms one-pole, and into the cutoff both directly and through a diode-charged 147 ms capacitor, mixed by Resonance; the capacitor empties with a 200 ms time constant and its swell is weighted twice the pulse (both our reading) |
| Slide: 60 ms exponential glide, no new envelope | A note played while another is held glides there (**Slide** sets the time, 60 ms by default) without starting the envelopes; on the pads, accent is velocity above **Accent at** |
| 3 ms soft attack; 8 ms linear fall at the gate's end | The same |

Left for later (our reading): the second low resonance from the coupling capacitors, the unit-to-unit spread of the square, the sequencer itself (the MPFree sequencer and arpeggiator play the engine; a pattern editor with per-step accent and slide is a separate piece), and comparing against a recording of a real unit.

## Open source to learn from

| Project | What | License | Notes |
|---|---|---|---|
| [Open303](https://sourceforge.net/p/open303/code) | A full emulation by Robin Schmidt (2009–2010) | MIT (the engine, per [jc303](https://github.com/amadeusp/jc303)) | Not yet studied. MIT, so its ideas and values can be used with credit. |
| [jc303](https://github.com/amadeusp/jc303) | Open303 ported to JUCE, as CLAP, VST3, LV2 and AU | GPL-3.0 (Open303's engine part MIT) | A CLAP build could be a plugin, through the CLAP thread. |
| [tb303-circuit-model](https://github.com/johnnyhawk02/tb303-circuit-model) | A part-by-part circuit model, as a web synth and VST3/AU | AGPL-3.0 | Not studied. |
| [a1k0n/303](https://github.com/a1k0n/303) | Resynthesis experiments and filter measurements on a clone | None stated | Its findings are cited above; no code taken. |

As with the [plugin pack](../design/plugins.md), GPL or AGPL code would ship only as a separate plugin, on the owner's go.
