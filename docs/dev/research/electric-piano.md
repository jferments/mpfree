# Electric pianos: what the research says, and open source to learn from

**Status:** research, 2026-11-14. Feeds the [Keys engine](../design/synths.md#keys-mpfree-synth-keys). Nothing here has run on the MPC.

The owner asked for electric pianos researched from papers and journals before they were built, because they are hard to get right. This page collects what the measurements and models in the literature say about the two classic designs (a struck steel tine beside a magnetic pickup, and a struck steel reed in an electrostatic pickup), what that means for a real-time engine on the MPC One, and which open-source projects are worth studying. Where a point comes from a source it is linked; where it is our own reading it says so.

## What makes the sound

### The vibrating part is nearly a sine; the pickup makes the tone

High-speed camera measurements of both instruments find the tine and the reed move almost sinusoidally, while the signal after the pickup is much more complex. The authors conclude the timbre is shaped mainly by the pickup, with the mechanical part secondary ([Pfeifle & Münster, DAGA 2017](https://pub.dega-akustik.de/DAGA_2017/data/articles/000210.pdf); [Münster & Pfeifle, ISMA 2014](http://www.conforg.fr/isma2014/cdrom/data/articles/000062.pdf)). Upper partials of the tine itself appear only in the attack.

### The tine's own modes: the bell of the attack

- The tine is a cantilever; its overtones are inharmonic and die fast, after which it settles to its fundamental ([Shear, UCSB 2011](https://www.mat.ucsb.edu/Masters/GregShearMasters2011_12_5.pdf)).
- Measured ratios of the second and third modes to the fundamental: **7.11 and 20.25** (sharp of a free bar's 6.27 and 17.55 because of the tuning spring's mass), as cited in [Sønderbo, AAU 2024](https://projekter.aau.dk/projekter/files/719175589/Master_Thesis_Real_time_physical_model_of_the_Rhodes_Electric_Piano_Tobias_Sonderbo.pdf).
- The tone bar (the other prong of the asymmetric tuning fork) follows the tine's frequency rather than its own, in phase or in antiphase, and adds a short burst of overtones for 10 to 14 ms after the strike; its inharmonic partials are most noticeable in the upper register (ISMA 2014, DAGA 2017).
- Gabrielli, Cantarini, Castellini and Squartini measured the inharmonic modes that shape the attack with a scanning laser vibrometer across the whole keyboard, and showed the magnetic pickup *intermodulates* them ([JASA 148(5), 2020](https://iris.univpm.it/handle/11566/286030)). We could read only the abstract; their per-key mode table is the best source for the next refinement.

### Sustain: long, and shorter up the keyboard

Shear measured Q factors on a tine piano: 949 at E-flat 2 (77.8 Hz), 731 at E-flat 3, 1520 at E-flat 4, 2175 at E-flat 5, 1761 at E-flat 6. With T60 = 6.9 Q / (π f), that is about 27 s, 10 s, 11 s, 8 s and 3 s. Our reading: T60 falls roughly as f^-0.75 across the range, with an unevenness from key to key.

### The magnetic pickup: a bell-shaped field, heard through its change

- The pickup is a coil on a magnet with a shaped pole piece. By Faraday's law its voltage is the rate of change of the flux, and the flux is a non-linear, roughly bell-shaped function of where the tine is (Shear 2011; DAGA 2017 FEM fields).
- **Voicing:** with the tine centred on the pickup the output is symmetric and the octave above the note dominates; moving it off centre brings back the fundamental and an asymmetric waveform (DAGA 2017, Shear 2011). Falaize and Hélie's simulations show the same: 0 mm offset gives the octave, 0.5 mm the familiar sound, 1.5 mm a pure tone ([Falaize, Rhodes simulation](https://afalaize.github.io/posts/rhodes/); [J. Sound Vib. 2016](https://www.sciencedirect.com/science/article/abs/pii/S0022460X16306320)).
- **Bark:** at large deflections the tine reaches the edges of the field, which gives the growl; it is mostly a low-register effect because low tines swing wider (ISMA 2014).
- **A good, cheap form:** for guitar pickups, measured responses reduce to a *Hammerstein* model: a static non-linear curve of displacement, then a time derivative ([Novak et al., Applied Sciences 2017](https://www.mdpi.com/2076-3417/7/1/50)). Gabrielli et al. model the Clavinet's pickups the same way, a polynomial of displacement then a differentiator ([EURASIP JASP 2013](https://asp-eurasipjournals.springeropen.com/counter/pdf/10.1186/1687-6180-2013-103.pdf)).

### The hammer

- Contact is modelled with the Hunt and Crossley law, force = k·x^α (1 + μ·ẋ), α about 2.8 for the tine piano's neoprene tip (Sønderbo 2024; Pfeifle, [DAFx 2017](https://www.dafx17.eca.ed.ac.uk/papers/DAFx17_paper_79.pdf)).
- A camera measured the contact at 6.42 ms on average (ISMA 2014). Harder hits give a shorter contact and more upper partials (DAFx 2017). Our reading: for a contact law of that exponent, contact time scales as v^-(α-1)/(α+1), about v^-0.47.

### The reed piano

- The reed is a cantilever whose pitch is set by solder at its tip; its higher modes are small and the fundamental dominates (DAFx 2017).
- The pickup is a capacitor: a charged plate beside the grounded reed, i = U·dC/dt, with C rising as the gap closes; the reed is not centred, so the capacitance changes differently in each direction (DAGA 2017).
- Measured: non-exponential decay and beating in the higher partials (DAGA 2017).
- The open-source model OpenWurli credits the capacitive 1/(1 - y) curve as the main source of the bark, followed by the RC load's high-pass, and models the preamp with its tremolo working inside the feedback loop, so the tremolo moves the tone with the level (fixed near 5.6 Hz) ([OpenWurli](https://github.com/hal0zer0/openwurli)).

### Amplification and tremolo

- The tine piano's stage amplifiers had a stereo "vibrato": first a square-wave tremolo, later a pan between the speakers; the matching preamp is described as dark with a slight edge of overdrive ([fenderrhodes.com, effects](https://www.fenderrhodes.com/history/effects.html)).
- Transistor stages add a few per cent of distortion at loud playing (the Clavinet preamp measured 1 % to 3.6 % THD, EURASIP 2013).

### Making it sound alive

- No two strikes are identical: OpenWurli adds a small per-mode frequency jitter "to avoid digital coherence"; real instruments are also voiced key by key.
- Beating and two-stage decay, which matter on acoustic pianos, are modelled cheaply with pairs of slightly mistuned two-pole resonators on the few partials where they are audible ([Bank et al., "Accurate and efficient modeling of beating and two-stage decay"](https://scispace.com/pdf/accurate-and-efficient-modeling-of-beating-and-two-stage-1xcli5evy4.pdf); [Bank, thesis](https://home.mit.bme.hu/~bank/thesis/)).

## What the Keys engine takes from this

| Finding | In Keys |
|---|---|
| Motion nearly sinusoidal, upper modes only in the attack | Per voice: the fundamental plus the 7.11 and 20.25 bending modes (tine), with short decays |
| Pickup = non-linear curve, then its change | `flux(u) = 1/(1+u²)` (tine) or `capacitance(y) = 1/(1-y)` (reed), then the sample-to-sample difference, divided by the note's angular frequency |
| Centred = octave, off centre = pure, between = bark | **Voicing** moves the rest point from the field's axis to its inflection (1/√3, the straightest point) |
| Low tines swing wider and bark sooner | The swing grows as (261.6 Hz / f)^0.5, limited to 0.5 to 1.6 |
| Pickup intermodulates the modes | Falls out of putting the sum of the modes through the curve |
| Sustain about T60 ∝ f^-0.75 | `decay_for` uses that slope, Decay sets middle C |
| Contact shorter when harder | Half-sine contact of 0.4 to 4.5 ms at full velocity, scaled by v^-0.47, never over half the period |
| Reed: two-stage decay, beating upper partials, RC high-pass, asymmetric preamp with tremolo inside | Two fundamental resonators (long and short decay), a 0.1 % mistuned pair on the second mode, a 120 Hz high-pass, an asymmetric preamp curve with the Amp tremolo before it |
| Strikes and keys differ | 0.3 % / 1.3 dB random detune and level per upper mode per strike; a fixed small Voicing offset per key |

Left for later (our reading of where the remaining realism is): the tone bar's own inharmonic burst in the upper register and its key-by-key mode table (needs the JASA 2020 data); a nonlinear Hunt and Crossley contact instead of the half-sine; the damper's felt as a nonlinear spring with its own release sound; a fuller amplifier and speaker model; 2× oversampling of the pickup curve for hard low notes.

## Open-source pianos

All of these are GPL or AGPL, so MPFree (MIT) can study them but not copy code. Like the [plugin pack](../design/plugins.md), any of them would ship as a separate CLAP file with its sources, built from source only on the owner's go.

| Project | What | License | Formats | Notes |
|---|---|---|---|---|
| [OpenWurli](https://github.com/hal0zer0/openwurli) | Reed piano: 7-mode reed, 1/(1-y) pickup, preamp solved from the circuit's netlist (DK method), tremolo oscillator, power amp, speaker | GPL-3.0 | CLAP, VST3 | Rust; the most thorough reed model found; calibrated against recordings |
| [electropiano](https://github.com/mtyas/electropiano) | Tine piano (port-Hamiltonian, after Falaize and Hélie) and a C++ port of OpenWurli, pickups and amps swappable | GPL-3.0 | CLAP, VST3, LV2 | Very new (6 commits) |
| [epi](https://github.com/DatanoiseTV/epi) | Tine, reed, electric grand, grand, clav, all modelled | GPL-3.0 | CLAP, VST3, AU | JUCE, plain C++ DSP headers; a headless build runs on arm64; early, single author |
| [Unsampled Piano](https://github.com/mgth/unsampled-piano) | Acoustic grand: stiff strings with two polarisations, felt hammer, modal soundboard, radiation, room | GPL-3.0 | CLAP, VST3 | Rust; quality presets; Linux x86_64 builds only so far |
| [OpenPiano](https://github.com/michele-perrone/OpenPiano) | Acoustic piano: finite-difference stiff string and hammer | AGPL-3.0 | JUCE app/plugin | Alpha; heavy for the MPC |
| [EP-MK1](https://github.com/MikeMorenoDSP/EP-MK1) | Tine piano in Pure Data (Camomile) | see repo | VST | A study piece |
| [Salamander Grand Piano](https://sfzinstruments.github.io/pianos/salamander) | A sampled Yamaha C5 (SFZ) | CC-BY 3.0 | samples | For an acoustic piano through the sampler, if SFZ import is added |

For an acoustic piano on the MPC One, our reading is that a sampled grand (Salamander) through MPFree's own sampler, or Unsampled Piano's `light` preset built for ARM, are the realistic options; a full physical model is likely too heavy for the RK3288. Both need the owner's decision.
