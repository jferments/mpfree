# Wind instruments: the clarinet, the sax and the flute as waveguides

**Status:** research, 2026-11-19 (synth library wave 2, the Wind engine). Deepens the winds part of [Organs, winds, choirs and tape](organ-wind-choir.md), which proposed the targets O3 (breath) and O4 (tuning) and the knobs players expect. The engine built from it is [Wind](../design/synths.md#wind-mpfree-synth-wind). Plan: "Synth library: stock-take and research plan" (https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). Nothing here has run on the MPC.

It answers the same eight questions as the other synth research pages. Where a point comes from a source it is linked; where it is our own reading, arithmetic or a measurement from our prototypes, it says so. The owner chose pure synthesis: no recorded instruments.

## 1. What makes a great one

- **The breath is the instrument.** A wind note is a valve (a reed, or an air jet) turning a steady breath into an oscillation that the air column holds. Blowing harder makes it both louder and brighter: soft playing stays in the valve's near-linear range and is nearly sinusoidal, and harder blowing clips the flow and adds the high harmonics the ear is most sensitive to ([UNSW, clarinet acoustics](https://newt.phys.unsw.edu.au/jw/clarinetacoustics.html)). A flute's soft C4 is "dominated by its fundamental" and louder playing adds harmonics ([UNSW, flute acoustics](https://newt.phys.unsw.edu.au/jw/fluteacoustics.html)). A wind synth whose level moves but whose tone doesn't sounds like an organ with a volume pedal (our reading).
- **The bore picks the harmonics.** A closed cylinder (the clarinet) supports the odd harmonics in its low register, so its spectrum has strong 1st and 3rd and weak 2nd and 4th harmonics, and it overblows a twelfth; a cone (the sax) sounds all harmonics and overblows the octave ([UNSW, clarinet](https://newt.phys.unsw.edu.au/jw/clarinetacoustics.html); [HyperPhysics, the saxophone](https://hyperphysics.gsu.edu/hbase/Music/sax.html)). The flute, a pipe open at both ends, also sounds every harmonic ([UNSW, flute](https://newt.phys.unsw.edu.au/jw/fluteacoustics.html)).
- **Notes start from noise and lock in.** Blowing a physical model softly, the breath noise "gradually grow[s] more and more resonant, until a pitched note locks into place" ([Sound On Sound, Yamaha VL1 review](https://www.soundonsound.com/reviews/yamaha-vl1)). Too little pressure gives only air noise; too much shuts the reed ([UNSW, clarinet](https://newt.phys.unsw.edu.au/jw/clarinetacoustics.html)). The start of a note, tongued or slurred, is a large part of what makes it sound played (our reading).
- **In tune while the tone moves.** A waveguide's pitch is its loop delay, and every filter in the loop adds phase delay that moves with the tone, so the length has to take the filter's delay off ([STK `Clarinet::setFrequency`](https://grok.dragonflybsd.org/xref/dports/audio/stk/stk-4.6.2/src/Clarinet.cpp?a=true): the delay is half the period less the filter's phase delay at the note).
- **Played legato, one note at a time.** The VL1, the classic physical-modelling wind synth, plays two notes at most and most patches are monophonic ([Sound On Sound](https://www.soundonsound.com/reviews/yamaha-vl1)).

## 2. How musicians use it

| Instrument | Jobs | Genres (examples) |
|---|---|---|
| Clarinet | Melodies, counter-lines, warm low parts | Film, klezmer, jazz, chamber pop |
| Sax | Leads, riffs, breathy ballad lines, growled solos, section stabs | Jazz, funk, soul, pop, lo-fi, house hooks |
| Flute | Melodies, breathy loops, ornaments, pan pipe and recorder colours | Lo-fi hip-hop, soundtrack, world, folk, ambient |
| Sections | Unison or chord pads from several reeds or flutes | Funk and soul sections, film |

(Our reading of the [research survey](organ-wind-choir.md#2-how-musicians-use-it).)

## 3. Algorithms

- **The digital waveguide** (Smith; the STK by Cook and Scavone): the bore as a delay line with a lossy, inverting reflection at the open end (a low-pass so the top decays faster), excited by a nonlinear valve driven by the mouth pressure. A pipe open at both ends has its fundamental at 1/(NT) with N the loop's delay in samples ([CCRMA, the virtual flute](https://ccrma.stanford.edu/realsimple/vir_flute/Traveling_Waves_Flute.html)).
- **The single reed.** The pressure across the reed is `Δp = Pm/2 − Pb+`, half the mouth pressure less the wave coming back from the bore, and the reed reflects the wave by a coefficient `ρ(Δp)` read from a table ([CCRMA, PASP, single-reed implementation](https://ccrma.stanford.edu/~jos/pasp/Digital_Waveguide_Single_Reed_Implementation.html)). The STK clarinet's table is a straight line with offset 0.7 and slope −0.3; Reed stiffness moves the slope from −0.44 to −0.18; breath noise is 0.2 of the pressure; vibrato 5.735 Hz at 0.1 ([STK `Clarinet.cpp`](https://grok.dragonflybsd.org/xref/dports/audio/stk/stk-4.6.2/src/Clarinet.cpp?a=true)). The flow through the reed rises with the pressure across it, then falls as the reed bends shut; the reed plays on the falling side ([UNSW, clarinet](https://newt.phys.unsw.edu.au/jw/clarinetacoustics.html)). In units of the pressure that closes the reed, oscillation starts at 1/3 for a lossless massless model and sustained oscillation stops at 1, where the reed stays shut ([Silva, Kergomard, Vergez and Gilbert](https://ar5iv.labs.arxiv.org/html/0810.2870)). Smith's reed table (`ρ = 1 − m (hc − h)`, clamped to 0 and 1, `m = 1/(hc + 1)`) is our form of that line, with `hc` the closing point.
- **The cone.** The STK's sax ("faux conical bore") puts the reed between two delay lines, a bidirectional waveguide whose "blowing position" sits between the terminations, with breath noise and vibrato multiplying the pressure and a one-zero loss filter scaled by −0.95 ([STK `Saxofony.h`](https://ccrma.stanford.edu/software/stk/Saxofony_8h_source.html)). Scavone's thesis models conical bores with waveguides and a memoryless reed ([Scavone, McGill course notes](https://www.music.mcgill.ca/~gary/618/week10/node13.html)). Our reading: a cone's input impedance is that of a pipe open at both ends driven a little way in, the missing tip being the short side, so the reed sees two lines in parallel.
- **The flute's jet.** The air jet moves at 20 to 60 m/s and a wave on it travels at roughly half that; it is deflected into and out of the hole by the field in the pipe, and reinforces the note when its transit time matches; players raise the pitch of the jet's "lock" by blowing harder or moving the lips closer ([UNSW, flute](https://newt.phys.unsw.edu.au/jw/fluteacoustics.html)). Both ends of the flute invert the wave ([CCRMA, virtual flute](https://ccrma.stanford.edu/realsimple/vir_flute/Traveling_Waves_Flute.html)). Our model: a jet delay of half a period, a smooth curve (`tanh`) for how much of the jet goes in, aimed off centre by Edge.
- **Brass** (not built): the lips as a valve, Bernoulli flow through them, one- and two-mass lip models (Rodet and Vergez), Cook's "swinging door"; the STK's `Brass` uses a second-order resonator, squared and clipped ([CCRMA, PASP, lips and mouthpiece](https://ccrma.stanford.edu/~jos/pasp05/Modeling_Lips_Mouthpiece.html)). A lip resonance that has to track the note is a second loop to tune and keep stable; we leave it for later (our reading).
- **Other ways** (not chosen): FM woodwinds and subtractive winds ([survey](organ-wind-choir.md#3-algorithms)). A waveguide plays from the breath by its nature, which is the point of the engine.

## 4. What to measure

From [Organs, winds, choirs and tape](organ-wind-choir.md#4-what-to-measure), the two rows that apply, with the shared rows every engine meets ([Synth quality](../design/synth-quality.md)):

| Id | Measure | Target | Why |
|---|---|---|---|
| O3 | Breath | Level and brightness rise monotonically with the breath; the note speaks within 30 ms at full breath | Winds that play |
| O4 | Wind tuning | Within 3 cents across the range | In tune |
| T1, L2, L3, L5, L6, L11, A1 | Tuning within 1 cent, a chord under 0 dBFS, DC, tone with velocity, the level law, level across the keys, aliasing | As every engine | The shared bar |

## 5. Knobs and defaults

The survey's list: Instrument (clarinet, flute, sax, brass), Breath source (pressure, velocity, CC 2), Reed or jet, Noise, Vibrato (rate, depth, delay), Brightness; pressure as breath ([survey](organ-wind-choir.md#5-knobs-and-defaults)). The STK clarinet maps Reed stiffness, Noise, Vibrato rate and depth and the breath to controllers ([STK `Clarinet.cpp`](https://grok.dragonflybsd.org/xref/dports/audio/stk/stk-4.6.2/src/Clarinet.cpp?a=true)). A harder reed "resists closing and allows harder blowing, but it sounds mellower and is less loud, all else equal" ([UNSW, clarinet](https://newt.phys.unsw.edu.au/jw/clarinetacoustics.html)).

## 6. The presets players expect

Clarinet, flute, breathy flute, alto sax-ish ([survey](organ-wind-choir.md#6-the-presets-players-expect)); from the jobs above (our reading): warm, jazz and dark clarinets; tenor, soprano, breathy and growled saxes; concert and breathy flutes, pan pipes, recorder, ocarina, bamboo flute; a reed section and a flute choir; a breath-controller patch and a pressure swell. Names are generic, never a maker's or a player's.

## 7. Playing it on the MPC

- **Velocity, then pressure.** The MPC's pads send velocity and per-note pressure; a pad's velocity sets the breath and its pressure adds to it through the shared shaper ([Pad pressure](../design/synths.md#pad-pressure-poly-aftertouch)).
- **A breath controller, the mod wheel or a foot pedal** on MIDI in: the VL1 was played with a breath controller, and the reviewer notes that foot controllers, velocity and aftertouch work but "require a great deal of practice" ([Sound On Sound](https://www.soundonsound.com/reviews/yamaha-vl1)). Instruments hear MPFree's controller events on MIDI in (`Ctl::Breath`, `ModWheel`, `Foot`).
- **Q-Links:** breath, vibrato, tone and noise on a held note.

## 8. What goes wrong

- A note that can't start from a soft breath, or starts with a click (our prototypes: a bore left to build up from nothing takes about 60 ms at C4 and four times that at C2).
- Tuning that drifts as the tone or breath moves: the loss filter's phase delay moves with its corner, and the valve adds its own with the breath (Wind takes off the first by formula and the second from a measured table).
- A reed that stops abruptly as the breath fades (a hard gate on the breath).
- A bright reed on a high note aliasing: the reed's hard corners feed harmonics past the output's Nyquist (our measurement, the clarinet at Tone 100: the worst alias 52 dB under the note at C7 with hard corners, 83 dB rounded).
- A sax that jumps to its octave when the cone's tip is too short (measured in our prototype).

## What MPFree has today

- **Wind** ([synths.md](../design/synths.md#wind-mpfree-synth-wind)), built 19 Nov: the clarinet, the sax and the flute on the voice core, 20 presets.
- Not built: brass (see Algorithms), the oboe and bassoon's double reeds, tone holes and register keys (the bore is one length per note), and overblowing to a higher register.

## Sources

- J. O. Smith, Physical Audio Signal Processing, single-reed implementation: https://ccrma.stanford.edu/~jos/pasp/Digital_Waveguide_Single_Reed_Implementation.html
- J. O. Smith, Physical Audio Signal Processing, lips and mouthpiece: https://ccrma.stanford.edu/~jos/pasp05/Modeling_Lips_Mouthpiece.html
- CCRMA REALSIMPLE, the virtual flute, traveling waves: https://ccrma.stanford.edu/realsimple/vir_flute/Traveling_Waves_Flute.html
- STK (P. R. Cook and G. P. Scavone), `Clarinet.cpp` 4.6.2: https://grok.dragonflybsd.org/xref/dports/audio/stk/stk-4.6.2/src/Clarinet.cpp?a=true
- STK, `Saxofony.h`: https://ccrma.stanford.edu/software/stk/Saxofony_8h_source.html
- G. Scavone, McGill course notes on conical bores: https://www.music.mcgill.ca/~gary/618/week10/node13.html
- F. Silva, J. Kergomard, C. Vergez and J. Gilbert, "Interaction of reed and acoustic resonator in clarinet-like systems": https://ar5iv.labs.arxiv.org/html/0810.2870
- UNSW Music Acoustics, clarinet acoustics: https://newt.phys.unsw.edu.au/jw/clarinetacoustics.html
- UNSW Music Acoustics, flute acoustics: https://newt.phys.unsw.edu.au/jw/fluteacoustics.html
- HyperPhysics, the saxophone: https://hyperphysics.gsu.edu/hbase/Music/sax.html
- Sound On Sound, Yamaha VL1 review: https://www.soundonsound.com/reviews/yamaha-vl1
- [Organs, winds, choirs and tape](organ-wind-choir.md) and its sources
