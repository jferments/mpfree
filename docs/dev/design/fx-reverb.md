<!--
covers: rust/mpfree-fx/src/rooms.rs rust/mpfree-fx/src/cloud.rs rust/mpfree-fx/src/oversample.rs
checked: 2026-11-17 dcbbe51
-->
# Reverbs: Room, Plate, Spring and the Cloud engine

**Status:** built; FX quality PR B (16 Nov: the Mix curve, the Room's level) and PRs E1 to E4 (17 Nov: the Room, the Plate and the Spring rebuilt, the Cloud engine reworked) are in. The engines the rebuilds replaced are under [History and next](#10-history-and-next). 113 of 113 measured reverb targets are met.

This page is the design of MPFree's reverbs as one effect type: what a reverb is for, the ideas underneath it, the research behind ours, which algorithms each engine uses and why, how quality and cost are measured, and how the defaults and presets were chosen. The parameter-by-parameter summary lives in [fx.md](fx.md) (the Room, Plate, Spring and Cloud reverb entries); the measured targets and their history in [fx-quality.md](fx-quality.md#reverb). The research notes, plan and baselines are in jferments/mpfree-claude `fx/quality/` (research-reverb.md, plan.md section 7, baseline-2026-11-16.md).

| Mode (Reverb family) | Engine, node id | Code | One line |
|---|---|---|---|
| Room | Room, `mpfree.fx.room` | [`rooms.rs`](../../../rust/mpfree-fx/src/rooms.rs) | Early reflections on the source's side, then a dense 16-line network; a Gate for the 1980s snare |
| Plate | Plate, `mpfree.fx.plate` | `rooms.rs` | Dense from the first millisecond, bright, long; the same network at a plate's lengths |
| Spring | Spring, `mpfree.fx.spring` | `rooms.rs` | Two springs of chirping all-passes, with Drip for the dub splash |
| Hall, Cloud, Echoes | Cloud reverb, `mpfree.fx.cloud` (Character 2, 3, 4) | [`cloud.rs`](../../../rust/mpfree-fx/src/cloud.rs) | A port of Cloud Seed: a slow, wide hall, an endless ambient wash, and distinct echoes that blur |

The Cloud reverb is also its own effect, with all five characters (Plate, Room, Hall, Cloud, Echoes), Freeze and a Quality choice. Two smaller reverbs live elsewhere and are not covered here: Perform FX's **Wash throw** (a Freeverb-style reverb of four damped combs and two all-passes a side, about 1.5 % of a period) and XY FX's **Wash** (the Plate at 100 % wet); fx.md describes both.

## 1. What a reverb is for

A reverb puts a sound in a space, or makes a space that never existed. Musicians reach for a few kinds, each with its own job and its own test of quality (research-reverb.md section 1):

| Kind | Typical decay | Pre-delay | Used for | What matters most |
|---|---|---|---|---|
| Room | 0.2 to 1 s | 0 to 15 ms | Drums, glueing a kit, making dry samples sound recorded | Early reflections; a dense, quick tail with no flutter; short decays that still sound smooth |
| Plate | 1 to 3 s | 0 to 80 ms | Vocals, snares | Dense from the first moment, bright and smooth, no distinct early reflections |
| Hall | 1.8 to 4 s | 10 to 40 ms | Strings, pads, ballads | A slow, smooth build; wide stereo; a warm top |
| Spring | 1 to 3 s | none | Guitars, organs, dub | The chirp ("boing") and the drip are the point |
| Ambient | 5 to 20 s, or frozen | any | Pads, transitions, sound design | Modulation, no metallic ring at long decays, Freeze |
| Gated | 0.2 to 0.5 s of hold | 0 | The 1980s snare | A dense burst that stops hard |

The usual moves, which the controls and presets serve: pre-delay keeps the dry attack clear of the reverb (20 to 40 ms on vocals); the reverb itself is filtered (the "Abbey Road" trick cuts its send below about 600 Hz and above 10 kHz); shorter and brighter feels closer, longer and darker pushes a sound back; and a shared reverb sits on a send return at 100 % wet, while an insert reverb runs at 10 to 30 % Mix.

On the MPC the reverb is also the most expensive effect: one reverb is about the whole effects budget (section 8), so every design here is chosen as much for its cost as for its sound.

## 2. Principles

**A reverb is many echoes that turn into noise.** A room's response is the direct sound, a few distinct early reflections from the nearest walls, then a late tail so dense that it is heard as decaying noise with the room's colour. Artificial reverbs build that from delay lines. Three properties decide whether one sounds natural:

1. **Echo density.** The number of echoes per second must grow fast enough that the ear stops hearing them one by one; sparse echoes are heard as flutter or grain, worst on drums and clicks. Abel and Huang's normalised echo density is 1 for Gaussian noise; the time to reach it is the "mixing time", tens of milliseconds in real rooms and growing with size (Lindau, Kosanke and Weinzierl 2012). Schroeder's rule of thumb asks for at least 1000 echoes a second.
2. **Mode density.** In frequency, a reverb is a forest of resonant modes. When they are too few, or when some decay slower than their neighbours, single notes ring out: the "metallic" sound of a cheap reverb. A network's mode density is the sum of its line lengths in seconds (modes per hertz), and Schroeder's rule of thumb asks for at least 0.15 × T60 of it: about 0.3 s of delay in total for a 2 s decay.
3. **One decay rate for every mode at a given pitch.** Every path round the loop must lose the same number of decibels per second, or the slowest one sets how long the tail lasts and its pitch rings on.

**Feedback delay networks** (Stautner and Puckette 1982; Jot and Chaigne 1991) are the standard way to get all three. N delay lines feed back through a mixing matrix that is unitary (it keeps energy), so the network without loss would ring forever and every mode would have the same, zero, loss. Loss is then added inside each line in proportion to its length: a line of `m` frames gets the gain `10^(-3 m / (fs T60))`, so every path loses 60 dB in T60 seconds whatever lines it took. To make highs die sooner, as in real rooms, that gain becomes a filter whose loss at each frequency is set from the T60 wanted there (Jot's absorption filters); an error in that filter shows directly as an error in the decay (Schlecht and Habets 2017). Mutually prime, well spread line lengths keep echoes from landing together; a dense matrix (Hadamard or Householder) mixes every line into every other on each pass, so the echo count multiplies.

**Diffusion before the network.** A burst entering the network as one click takes several passes to become dense. A few short all-passes in series first (Schroeder's all-pass, `y = -g x + x[n-d] + g y[n-d]`, flat in magnitude) smear it into a cluster, so the tail is dense from its first pass. They must stay short and few: several short all-passes in series ring metallically on voices (ValhallaDSP).

**Modulation, a little.** Slowly moving the length of a few long lines keeps modes from standing still, which hides what ringing is left (Stautner and Puckette). Too much is heard as chorus: pitch wobble on a held piano or organ note is the classic tell of a cheap reverb. Our target is under 3 cents RMS on a held note.

**Stereo from the structure, and the source's side kept.** The two outputs take different combinations of the lines, so they are decorrelated (low interaural cross-correlation, IACC) and sum to mono about 3 dB down without combing. A good reverb also keeps a source's place: a sound on the left should have its first reflections on the left (the precedence effect puts the source where the first sound comes from).

**Decay is a number the listener can hear.** The just noticeable difference for reverberation time is about 5 % (ISO 3382-1, as tabulated by Christensen and Rindel), so a Decay knob 20 % off is heard as "not what it says". By room-acoustics convention, decay is quoted at mid frequencies, the mean of the 500 Hz and 1 kHz octaves' T60, and so is ours.

**Level should not follow Decay or the mode.** Switching Room to Hall at the same Mix should not jump, and turning Decay up should make the tail longer, not louder. Each engine scales its input by what its loop keeps and is trimmed so its defaults and presets play within 1 LU of the dry at 100 % wet.

## 3. Research

Research for this class (16 Nov 2026, research-reverb.md) covered the acoustics literature (ISO 3382-1 and its just noticeable differences; Schroeder; Moorer; Jot; Abel and Huang; Lindau; Schlecht and Habets; Välimäki et al.'s "Fifty years of artificial reverberation"), the published designs we build on (Dattorro's plate, Välimäki, Parker and Abel's spring, Ghost Note Audio's Cloud Seed), designers' notes (ValhallaDSP on diffusion and metallic artifacts) and how engineers use reverb (Sound On Sound; the Abbey Road EQ). Findings that shaped the design:

- **What can be measured, and how finely:** T60, EDT and T30 to 5 %, C50 and C80 to 1 dB, echo density by Abel and Huang's measure, colour as a narrow peak in the tail's spectrum, IACC, wobble in cents. That became targets R1 to R12 (section 7).
- **The pitfalls musicians name** (section 9): metallic tails, flutter, mud, wash, sibilance splash, chorusy wobble, level jumps, a Decay that isn't what the knob says, mono collapse, tails cut off, and CPU.
- **The features people expect:** types, Decay, Size, Pre-delay, Damping, low and high cut on the reverb, Early level, Diffusion, modulation, Width, Freeze, Ducking, a Gate. We had most of them; Ducking, Pre-delay sync, a bass multiplier and Freeze on Room and Plate are the gaps (section 10).
- **Where MPFree stood** (the 16 Nov baseline, 68 of 114 targets): the Room was the weakest. Its eight lines of 19 to 48 ms, fed directly with no input diffusion, sounded like a slap and a faint tail (EDT/T30 0.37), took 220 to 314 ms to turn dense, rang 4.9 dB over an ideal tail and played 4.9 LU under the dry, which is why Jay heard "very weak". The Plate took 167 ms to turn dense and zipped when Size moved; the Spring's Decay ran 31 % short at 6 s; no reverb but the Cloud engine kept a source's side. The Hall was best, but its Decay was not right for every pitch and its level rose with Decay.

The conclusion was that the Room could not reach its targets by tuning and had to be rebuilt as a proper feedback delay network with input diffusion; the Plate, which had the same density problem, became a second voice of that network; the Spring and the Cloud engine kept their designs and had their decay, level and smoothness fixed.

## 4. The engines

### The shared network: `Net` and its voices (Room and Plate)

From PR E1 (Room) and E2 (Plate). The Room and the Plate are one engine, `Net`, voiced by `RoomVoice` and `PlateVoice`: the lengths, early pattern, wobble and trim are the voice; everything else is shared.

- **Input diffusion:** each side's input goes through four short Schroeder all-passes of about 1.5 to 6 ms, different on each side. Their gain is the Room's **Diffusion** control (up to 0.75) and fixed at 0.75 on the Plate.
- **The late network:** sixteen delay lines mixed by a 16-point Hadamard matrix. A Hadamard matrix has every entry ±1, so every line feeds every other on every pass, and it is applied as a fast Walsh-Hadamard transform: 64 adds and subtracts, no multiplies (the 1/4 that makes it lossless is folded into each line's loss). The left input goes into the even lines and the right into the odd, with signs from a row of the matrix. The line lengths are about geometric, nudged off simple ratios so no two lines' echoes keep landing together: 15.3 to 74.3 ms for the Room at size 1, 32.2 to 152.6 ms for the Plate. In total that is 0.60 s and 1.25 s of delay, enough modes (by the rule of thumb in section 2) for about 4 s and 8 s of decay.
- **Loss:** each line has Jot's one-pole absorption filter, worked out from its own length (`room_loss`) so every line falls 60 dB in Decay at 707 Hz and in (1 - 0.9 Damping) times Decay at the top. The network's input is scaled by `sqrt(1 - g²)` at the mean line's gain, which keeps the tail's level steady as Decay changes.
- **Outputs:** each side is a blend of two of the transform's outputs (the left `0.75 s5 + 0.25 s4`, the right `0.75 s10 - 0.25 s11`), which works out as a row of the matrix that hears that side's own lines at full strength and the other side's at half. They are read off the transform for free, and different rows make two decorrelated sides.
- **Modulation:** the eight longest lines wobble, at 0.43 Hz and up, each 5 % faster than the one before; the others are read at whole frames. The diffusers never move (moving short all-passes is the "water in a metal pan" sound).
- **Early sound:** a stereo tapped delay after the diffusers, each tap heard strongly on its own side and less on the other, so a source keeps its side. The taps stay out of the network, because their comb would colour the whole tail.
- **Smooth controls:** Size glides over 300 ms and every read follows it at a fraction, then settles on whole frames over 3 ms; Pre-delay crossfades to its new time over 20 ms rather than sliding (a slide bends the pitch); the loss filters glide over 15 ms when Decay, Damping or Size move.
- **Chunks:** a block is worked through in chunks of up to 64 frames, a stage at a time (pre-delay, diffusers, taps, every line's reads, then the matrix a frame at a time, then every line's writes). That is possible because no line is shorter than a chunk at any rate (a unit test checks it), and it is what makes the network cheaper than the old one. The chunks fall on a fixed grid from reset and each makes its choices at its first frame, so the output is the same, bit for bit, at any block size, even while controls move. Every delay is one power-of-two ring written at one shared count.

### Room

Since PR E1 (17 Nov); the Room it replaced is in section 10.

The Room's voice: lines of 15.3 to 74.3 ms at size 1, with **Size** 0 to 100 % scaling them 0.25 to 1.5 times (size 1 is Size 60 %). **Early reflections** are twelve taps a side, 3 to 45 ms at size 1 and spread by Size, at different times on each side, each a short cluster rather than a click because it comes after the diffusers; **Early** sets their level. **Mod** wobbles the eight longest lines by up to 96 frames at 48 kHz (2 ms) at 100 %, 24 frames at the default 25 %. **Gate** cuts the tail that many milliseconds after the input stops, the 1980s gated snare. Pre-delay goes to 100 ms.

Why this design: the old Room's eight short lines fed directly could not be dense in time (220 to 314 ms to diffuse against a 50 ms target) or in frequency (4.9 dB over an ideal tail), and its reflections were mixed to the middle. Doubling the lines, adding the diffusers and keeping the taps out of the loop met every target at once (section 7). More wobble or longer lines bought little: all sixteen lines wobbling read 1.6 dB over the mean of four ideal tails with a 5-cent smear, and lines 1.6 times longer also 1.6 dB with fewer early echoes, against 1.65 dB for the chosen design, so this is about where a sixteen-line network sits at 0.8 s.

### Plate

Since PR E2 (17 Nov); Dattorro's tank it replaced is in section 10.

A plate has no walls, so it has no distinct early reflections; what it has is a dense onset that is there at once. The Plate's voice:

- **Tone**, the bandwidth filter (a one-pole low-pass), before the diffusers, as on a real plate's drive.
- **A dense onset:** twenty taps a side from 1 to 29 ms, the same at every Size (`Voice::EARLY_SIZED` is false), stronger on the source's own side, so a hit is dense from its first milliseconds, keeps its side and runs into the tail with no gap (a test checks there is no dip at Size 50, 100 and 150 %).
- **The network at a plate's lengths:** 32.2 to 152.6 ms at size 1; **Size** 50 to 150 % scales them 0.55 to 1.05 times. Long lines make many modes, so none rings out: with the Room's lengths the tail rang about 1 dB more (2.1 dB over the mean of four ideal tails against 1.2), and more wobble barely helped. With lines that long the first ones come back only after 18 to 34 ms, which is why the onset is a fixed 29 ms and Size spans 0.55 to 1.05 rather than 0.5 to 1.5.
- **Mod** wobbles the eight longest lines by up to 40 frames at 48 kHz at 100 % (16 at the default 40 %).
- Pre-delay goes to 250 ms (plates are often run with long pre-delays on vocals).

Why not Dattorro's figure-eight tank, the classic plate: it is a beautiful design, but in our measurements it took 167 ms to turn dense (a grainy start), rang at 3.7 kHz, summed a source to the middle, ran 15 % long on short decays and smeared a held note by 4.9 cents; and the shared network is 17 % cheaper.

### Spring

Since PR E3 (17 Nov); the version it replaced is in section 10.

A spring tank is a transducer, one or more coiled springs and a pickup. A torsional wave on a helical spring is strongly dispersive: high frequencies travel at different speeds from low ones, so each pass along the spring arrives as a falling or rising chirp, and the tank's echoes are chirps that repeat every round trip. Välimäki, Parker and Abel (2010) model that with a chain of identical first-order all-passes in a feedback loop; MPFree follows them.

- **Two springs, one a side**, the second 13 % longer, each hearing its own side and 0.35 of the other, so a source keeps its side and the two sides differ.
- **The chirp:** each spring passes the sound through a chain of 40 first-order all-passes in `z^-K` ("stretched": `(a + z^-K) / (1 + a z^-K)`). Stretching by `K` puts the chain's dispersion below `fs / 2K`; with `K` the rate over 8820 (5 at 44.1 and 48 kHz, 11 at 96 kHz) the chirp sits below about 4.4 kHz and lasts as long at every rate. **Tension** sets the all-passes' coefficient, the chirp's steepness. The paper uses 100 to 200 stages; 40 is what fits the MPC, and still chirps clearly.
- **The loop:** a delay with a loss for Decay, so each return chirps again; **Length** is the round trip. The loop's own damping is a fixed one-pole at 5 kHz.
- **Drip** wraps the chain in an all-pass (gain up to 0.3, hearing the chain 32 frames late) that sends some of each chirp round again: the dub splash. Because the loop stays an all-pass, Drip never changes Decay.
- **Tone** filters what you hear (a 12 dB/octave low-pass on the output), not what goes round, so it never changes how long the middle rings. **Low cut** thins the output.
- **Length is exact:** the delay is shortened by what the chain, Drip's wait and the damping add at 707 Hz, so a return comes back every Length there, as the Spring's card draws it.
- **Decay is solved, not guessed:** each octave's trip takes its own time (the chain delays lows more), and the damping takes its own loss, so the loss is the root of a quadratic in the loop's log loss, taken at 410 and 820 Hz, with Drip's longest trips added. Those two points, not the octaves' centres, because each octave's T30 is set by its slow end: solving at the centres left Decay 3 to 8 % long (28 % at Length 20 ms and full Tension), and at a single 560 Hz point 8 % short. Lows still ring on, as a spring's do.
- **Level** follows the loop's gain at 1 kHz, so a longer Decay rings longer rather than louder.
- **Cost:** the springs work 32 frames at a time, a stage at a time over the piece with four frames side by side, about a quarter cheaper than a frame at a time and the same, bit for bit, at any block size.

### The Cloud engine (Hall, Cloud, Echoes)

The Cloud reverb is a Rust port of the MIT-licensed [Cloud Seed](https://github.com/GhostNoteAudio/CloudSeedCore) reverb core by Ghost Note Audio (the notice is in [`LICENSE-cloudseed.txt`](../../../rust/mpfree-fx/LICENSE-cloudseed.txt); MPFree is not made or endorsed by Ghost Note Audio). Cloud Seed's path: input cross-mix, low and high cut, pre-delay, a multi-tap early pattern, a chain of modulated all-pass diffusers, then up to 12 modulated feedback lines, each with its own diffuser, shelves and damping in the loop. Every length and rate comes from a seeded random series that differs per side, so the tails are decorrelated. It is not a mixing-matrix network: each line feeds only itself, and density comes from the all-pass diffusers inside every loop. That makes it very dense and smooth, and expensive.

Its 45 parameters became 15. A **Character** picks the hidden structure (the Plate is Cloud Seed's Dark plate structure with a short early smear added):

| Character | Lines | Stages in each line's diffuser | Early diffuser stages | Early taps | Out | In-loop shelves |
|---|---|---|---|---|---|---|
| Plate | 12 | 4 | 4 | 0 | after the diffuser | high -4.6 dB at 4.5 kHz |
| Room | 8 | 3 | 4 | 24 over 40 ms | after | low -2 dB at 200 Hz, high -3 dB at 6 kHz |
| Hall | 10 | 4 | 6 | 40 over 90 ms | after | high -3 dB at 5 kHz |
| Cloud | 12 | 6 | 10 | 0 | after | high -2.5 dB at 6.5 kHz |
| Echoes | 4 | 2 | 0 | 12 over 600 ms | before the diffuser | low -3 dB at 250 Hz, high -5 dB at 3.5 kHz |

(Counts at Full quality and the reference Size.) Echoes is heard before its lines' diffusers, so its repeats stay distinct and blur as they fade.

**Quality** has three settings. Light runs half the lines (at least 4), half the stages, half the early diffuser and half the taps, with the same seeds, lengths, EQ and decay: the same size, colour, level (within 1.5 dB) and decay (within 6 %), a little less dense, for about 45 % of Full's cost. **Eco is the default** (Jay's pick, so one reverb fits the MPC's budget): Light's structure at half the node's rate, between polyphase IIR half-bands (six first-order all-passes each, flat to 0.19 of the node's rate and 85 dB down from 0.31, [`oversample.rs`](../../../rust/mpfree-fx/src/oversample.rs); unrolled as one `IirPair` a side, its down-sampler and up-sampler each flushed of tiny states every 16 of their frames, which took 3 to 4 % off Eco in DSP quality step 2.3B with the same output to the bit). Mix stays at the full rate, so the dry signal is untouched; the wet signal is about 7 frames late, and the half-bands' non-linear phase is inaudible on a reverb's wet signal. At half rate a cubic or linear read of a fraction would dull the top of the band on every pass, so Eco's stages and taps read fractions with a first-order all-pass (Thiran), which loses nothing. Eco keeps Light's colour within 2 dB from 1 to 10 kHz and has nothing above about 13 to 14 kHz, for about 30 % of Full's instructions.

What the port changed from Cloud Seed: feedback per frame (Cloud Seed feeds back one buffer late, so its output depends on the block size), deterministic modulator phases, cubic reads and whole-frame base delays (so Modulation 0 is lossless and Freeze holds without dulling), an O(1) reset, and every control glides. **Freeze** mutes the input and holds the tail; a frozen reverb's tail is endless, so it keeps ringing when bypassed.

Since PR E4 (17 Nov; before it, the port kept Cloud Seed's loss and level):

- **Decay for every pitch.** Cloud Seed loses a fixed share each pass. But an all-pass diffuser inside the loop delays some pitches far more than others (its group delay swings with its feedback), so with a fixed loss per pass those pitches ring longest and T30 follows the slowest: the Cloud character ran 22 % long at 2.5 s and Echoes' tail fell in steps. Now the loss is a rate per frame, spread over each line and inside each diffuser stage's feedback in proportion to its delay, so every path loses exactly in proportion to the time it takes, and the impulse response is the lossless one times one exponential. The rate is solved for the 500 Hz and 1 kHz octaves with the loop's shelves and damping counted in: `a = (K - T (m1 + m2) + sqrt(T² (m1 - m2)² + K²)) / 2T`, with `K = 3 ln 10`, `T` Decay in frames and `m` each octave's EQ loss over a trip (`loss_rate`, `mid_loss`).
- **A steady level.** Scaling the input by `1 / Σ 1 / (1 - pass)` holds the level at long decays but not at short ones, where a pass through a lossy all-pass keeps more than its delay suggests (the straight-through part keeps its `g²` whatever the loss): wide open, the Cloud read 9 dB down at 0.3 s. The lines' input now follows the energy each loop keeps for noise: each stage's own energy, `g² + loss² (1 - g²)² / (1 - g² loss²)` (`stage_energy`), the first pass out, over six octaves weighted as the loudness meter hears pink noise. At the defaults the wet loudness stays within 0.1 LU from 0.3 to 8 s (it rose 3.5 to 5.2 LU).
- **Size and Pre-delay crossfade** (25 and 20 ms) from the old lengths to the new, instead of sliding every length, which bent the pitch (a held 1 kHz tone was nearly all more than 60 Hz off while Size moved; now under a sixteenth of its energy). Each line, stage and tap reads both lengths during a fade. Modulation glides over 300 ms.
- **Taps after the early diffuser**, making the early sound; the lines take the diffused input, so the taps' colour does not tint the tail. The Hall's lines also take 0.2 of the taps (to 0.9 of the diffused input): at a big Size each pass round a line is a good part of a short decay, and fed one compact burst the tail came in clusters with gaps (46 % short at Size 600 ms and 1.5 s). Echoes' lines take the taps alone, so its pattern repeats.
- **Line modulation per character** (the Cloud's and Echoes' at half depth), so a held note smears under 3 cents at Eco, and every character's trim set anew.

## 5. Controls

Every engine has a **Mix curve** (Equal by default, Dry first, Linear; [fx.md](fx.md#the-delays-and-reverbs-mix-curve)) after its other controls, drawn as a small chip in the Output section rather than a row of buttons. Presets never change Mix, Mix curve or Quality.

| Engine | Page | Controls (range, default) |
|---|---|---|
| Room | Room | Pre-delay (0 to 100 ms, 5), Size (0 to 100 %, 45), Decay (0.1 to 5 s, 0.8), Damping (0 to 100 %, 50), Early (0 to 100 %, 60), Mod (0 to 100 %, 25), Gate (0 to 600 ms, 0 = off); Diffusion (0 to 100 %, 70; from PR E1, a trim tier control) |
| | Output | Low cut (20 Hz to 1 kHz, 60 Hz), Width (0 to 100 %, 100), Mix (0 to 100 %, 25), Mix curve |
| Plate | Plate | Pre-delay (0 to 250 ms, 10), Size (50 to 150 %, 100), Decay (0.2 to 20 s, 2), Damping (0 to 100 %, 35), Tone (1 to 20 kHz, 12 kHz), Mod (0 to 100 %, 40) |
| | Output | Low cut (20 Hz to 1 kHz, 80 Hz), Width (0 to 100 %, 100), Mix (0 to 100 %, 25), Mix curve |
| Spring | Spring | Length (20 to 80 ms, 40), Decay (0.5 to 8 s, 2.5), Tension (0 to 100 %, 50), Drip (0 to 100 %, 50), Tone (1 to 8 kHz, 4 kHz), Low cut (20 Hz to 1 kHz, 150 Hz), Mix (0 to 100 %, 30), Mix curve |
| Cloud | Reverb and Output | Character (Hall), Size (20 to 1000 ms, 120), Decay (50 ms to 60 s, 2.5 s), Pre-delay (0 to 500 ms, 10), Diffusion (0 to 100 %, 70), Modulation (0 to 100 %, 30), Mod rate (0.05 to 5 Hz, 0.3), Damping (400 Hz to 20 kHz, 9 kHz), Low cut (20 Hz to 1 kHz, 80 Hz), High cut (400 Hz to 20 kHz, 14 kHz), Width (0 to 100 %, 100), Early (0 to 100 %, 40), Mix (0 to 100 %, 30), Freeze (off), Quality (Eco), Mix curve |

What the controls mean, in the same words on every engine where they share a name:

- **Decay** is T60 at mid frequencies (the mean of the 500 Hz and 1 kHz octaves), the room-acoustics convention, not a feedback amount. On the Room and Plate the network's reference is 707 Hz, between the two.
- **Damping** on the Room and Plate is how much sooner the top dies: the top's T60 is (1 - 0.9 Damping) times Decay. On the Cloud engine it is a corner frequency in the loop, as in Cloud Seed.
- **Size** scales the line lengths (and, on the Room, the reflections); on the Cloud engine it is the reference length in ms.
- **Early** is the early reflections' level; **Width** blends the two outputs toward their middle (0 is mono); **Low cut** and **High cut** filter the wet signal only, the reverb-send EQ engineers would otherwise add by hand.
- **Mod** (Room, Plate) is depth only, with rates fixed per voice; the Cloud engine has Modulation and Mod rate.

A family mode carries every control by name when switched, so a Room switched to Plate keeps its Decay and Mix.

## 6. Defaults and presets

**Defaults** were set when each engine was first built (the Room, Plate and Spring with the MPC coverage effects on 14 Nov, the Cloud reverb in F2) from common studio practice, and checked against the research's ranges in this round. The quality work kept every default and preset value and changed only how they sound, so a saved project keeps its settings and sounds like the better reverb.

| Engine | Default | Why |
|---|---|---|
| Room | Size 45 %, Decay 0.8 s, Pre-delay 5 ms, Early 60 % | A medium room for drums: inside the 0.2 to 1 s and 0 to 15 ms the research found for rooms, with reflections clearly heard, which is what makes a dry kit sound recorded |
| Plate | Decay 2 s, Pre-delay 10 ms, Damping 35 %, Tone 12 kHz | A bright vocal and snare plate, in the 1 to 3 s plate range; Tone high because a plate's brightness is its point |
| Spring | Length 40 ms, Decay 2.5 s, Tension 50 %, Drip 50 % | A guitar-amp tank: an audible chirp and splash without the dub extreme |
| Cloud | Hall, Size 120 ms, Decay 2.5 s, Diffusion 70 %, Modulation 30 %, Quality Eco | A hall in its 1.8 to 4 s range; enough modulation to keep a long tail smooth, under the wobble target at Eco; Eco so one reverb fits the MPC's budget |
| All | Low cut 60 to 150 Hz, Width 100 % | The "EQ the reverb" advice, kept mild: a cut at 600 Hz (Abbey Road) is a mix decision, not a default; the presets go further where they need to |
| All | Mix 25 to 30 % on an insert, 100 % on a send return | The usual insert range (research: 10 to 30 %); on a return the effect should be all wet, since the send sets the amount, so a delay or reverb added to a return starts at Mix 100 % (Jay, 16 Nov) |
| All | Mix curve Equal | Equal power keeps the level as Mix turns: the linear crossfade made every reverb 2 to 2.4 dB quieter at its default (Jay, 16 Nov) |

**Levels.** Every engine's wet signal is trimmed so its defaults and every preset play within 1 LU of the dry at Mix 100 % on pink noise (R6), so switching mode or preset does not jump. Before PR B the Room played 4.9 LU under the dry, which, with the linear Mix law, is why the Room on the drums sounded "very weak".

**Presets** (a preset sets the sound, never Mix, Mix curve or Quality):

| Engine | Preset | Values | Use |
|---|---|---|---|
| Room | Small booth | Size 15 %, Decay 0.35 s, Early 80 % | A dry vocal booth; mostly reflections |
| | Drum room | Size 50 %, Decay 0.9 s, Damping 35 % | The kit in a live room |
| | Big studio | Size 85 %, Decay 1.8 s, Pre-delay 15 ms | A large tracking room |
| | Gated snare | Size 70 %, Decay 2.5 s, Gate 250 ms, Damping 20 % | The 1980s snare: a big, bright burst cut hard (0.2 to 0.5 s of hold in the research) |
| Plate | Vocal plate | Decay 1.8 s, Pre-delay 30 ms | The classic vocal plate (1.2 to 2.5 s, 20 to 40 ms pre-delay) |
| | Snare plate | Decay 1.2 s, Size 80 %, Damping 20 % | Short and bright for a snare |
| | Dark plate | Decay 3.5 s, Damping 70 %, Tone 4 kHz | A long, dark plate that sits behind a mix |
| | Endless plate | Decay 18 s, Size 140 %, Mod 80 % | An ambient pad plate |
| Spring | Amp spring | Decay 2 s, Drip 40 % | A guitar amp's tank |
| | Dub splash | Decay 4.5 s, Drip 90 %, Tension 70 % | The dub splash, all drip |
| | Short tank | Length 25 ms, Decay 1 s, Tone 6 kHz | A small, bright tank |
| Cloud | Dark plate | Plate character, Size 96, Decay 4.8 s, Diffusion 97 %, Low cut 340 Hz | Cloud Seed's dark plate |
| | Small room | Room, Size 40, Decay 0.7 s, Early 60 %, High cut 12 kHz | A small room |
| | Big hall | Hall, Size 180, Decay 3.5 s, Pre-delay 25 ms, High cut 11 kHz | A concert hall (1.8 to 4 s, 10 to 40 ms) |
| | Endless cloud | Cloud, Size 300, Decay 20 s, Modulation 60 %, High cut 10 kHz | The ambient wash |
| | Echo chamber | Echoes, Size 260, Decay 3 s, Low cut 200 Hz, High cut 8 kHz | Distinct, dark echoes that blur |

fxlab measures every preset's level, zipper and the other quick rows as it does the defaults (section 7): each is within 1 LU of the dry and zips under 3 dB.

## 7. Measuring quality

`fxlab quality --class reverb` measures every Reverb mode at its defaults and every preset, wet only (Mix 100 %), at 44.1 kHz, 113 numbers in all; the ratchet (`rust/mpfree-fxlab/quality/ratchet.tsv`, checked on CI) fails a pull request that makes any of them worse. The analyses (`rust/mpfree-fxlab`): `knob` (Decay against T60, from the Schroeder backward integral of each octave's impulse response; Pre-delay against when the reverb starts), `clarity` (C50, C80, EDT over T30, which side a reverb starts on), `wobble` (a held 440 Hz tone's spread in cents), the echo density and tail-excess measures, and `mixlaw` for loudness (BS.1770).

| Id | What | Target | 16 Nov baseline | With PRs E1 to E4 |
|---|---|---|---|---|
| R1 | T60 at mid frequencies against Decay | Within ±10 % | Room -21 % at 0.3 s; Plate +20 % at 0.5 s; Spring -31 % at 6 s; Cloud +22 % at 2.5 s; Echoes -27 % at 6 s | Room within 4 %, Plate 1.1 %, Spring 4.7 %, Hall 4.7 %, Cloud 2.4 %, Echoes 3.9 % |
| R3 | One slope: EDT over T30 | 0.8 to 1.25 (Hall and Cloud 1.6; Echoes reported) | Room 0.37 | Room 0.91, Spring 0.95, Hall 1.10, Cloud 1.39 |
| R4 | Time to turn dense (echo density 0.9) | Plate 30 ms, Room 50, Hall and Cloud 100 | Room 220 to 314 ms; Plate 167 to 202 | Room 18 ms, Plate 10 ms |
| R5 | Colour: tail excess over an ideal noise tail of the same T60 | ≤ 1.5 dB (Spring: excess ≤ 8 dB; Echoes reported) | Room 4.9 dB; Plate 2.75; Hall 2.23 | Room 1.26, Plate 0.38, Hall 0.99, Cloud 0.29; Spring 4.68 |
| R6 | Wet loudness against the dry on pink noise | ±1 LU, every mode and preset | Room -4.9 LU | All within 0.9 LU |
| R7 | Loudness as Mix turns | As D6 (-1 to +1.5 LU) | -1.9 to -2.4 LU dip | Under 1 LU (PR B) |
| R8 | Tail IACC; mono sum | IACC ≤ 0.2; mono within 3.5 LU | Met | Met (mono sums 2.3 to 3.0 LU down) |
| R9 | A held 440 Hz tone's smear | ≤ 3 cents RMS at default modulation | Plate 4.95; Hall 3.15; Cloud 3.55; Echoes 3.04 | Room 1.87, Plate 1.47, Hall 2.69, Cloud 1.86 |
| R10 | Source's side: first 50 ms from a left input | ≥ 3 dB louder on the left | Room, Plate, Spring summed to mono | Room 4.3 dB, Plate 6.0, Spring 9.0 |
| R11 | Zipper: the sharpest step while a control sweeps | ≤ 3 dB, every control and preset | Plate Size 5 to 7 dB; Echo chamber Size 21 dB | Room 2.4, Plate under 1, Spring 1.42, Hall 1.52, Cloud 2.86, Echoes 0.95 |
| R12 | Pre-delay | Onset within one frame | Room a frame off | Exact |

How the main measures work and why they were chosen:

- **Decay (R1)** is each octave's T30 (the slope of the Schroeder curve from -5 to -35 dB, times two), averaged over 500 Hz and 1 kHz. The 10 % target is twice the 5 % JND, so a knob that misses is still close to inaudible. The unit tests check further: the Cloud engine's octaves at 44.1, 48 and 96 kHz and Size 40 to 1000 ms (a line fitted from -5 to -60 dB, so a big Size's clustered passes read true).
- **One slope (R3)**: EDT (the first 10 dB) over T30. A ratio well under 1 means energy front-loaded, heard as a slap then a faint tail (the old Room's 0.37). Halls build slowly, so up to 1.6 is allowed; Echoes builds by design and is reported only.
- **Echo density (R4)** is Abel and Huang's normalised echo density (the share of samples outside one standard deviation in a sliding window, against Gaussian noise's), and the time at which it reaches 0.9. The targets follow the mixing times of real rooms: a plate dense at once, a small room within tens of milliseconds.
- **Colour (R5)**: the tail's averaged spectrum, its loudest narrow peak over its third-octave average, against the same measure on ideal exponentially decaying Gaussian noise of the same T60 (which reads 1.4 to 3.4 dB by itself, shorter tails reading higher). A spring is meant to ring, so it has its own ceiling.
- **Wobble (R9)** is the spread of a held tone's spectrum in the tail, in cents RMS: 0 for a still reverb, and the classic tell of cheap modulation when it is high.
- **Source's side (R10)** and IACC (R8) together: early energy on the source's side, a decorrelated tail.

What fxlab does not measure yet: R2 (does the top's T60 follow Damping's stated ratio), R13 (Freeze holds within 0.5 dB for 30 s; tails ring out after bypass, partly covered by unit tests), the characters at Full quality (fxlab runs Eco, the default: at Full a held note smears 3.7 cents on the Hall and 4.1 on the Cloud), and the Cloud reverb's own Plate and Room characters (unit tests check their level).

**Unit tests**: `tests/room.rs`, `tests/plate.rs` and `tests/spring.rs` (from PRs E1 to E3: the ids saved projects use; Decay at 44.1, 48 and 96 kHz across sizes and settings; Pre-delay moving the whole wet signal exactly; a source keeping its side; the tail ringing out; every control swept under noise without blowing up; bit-identical output at any block size), `tests/cloud.rs` (Decay per octave, every pitch fading together, Size and Pre-delay jumps not bending pitch, a longer Decay not louder, the source's side, Freeze, Width, Light and Eco against Full, the Eco change bit-exact across block sizes), `tests/mix_curve.rs`, and the registry-wide gates for block size, extremes, NaN and allocation.

**Listening:** each rebuild has a blind, loudness-matched A/B page (old against new on drums, vocals, guitar and pads) for Jay; numbers decide what is fixed, listening decides whether the fix is better.

## 8. Cost

The instruction gate (`mpfree-bench`, callgrind, rustc 1.97.0, a 128-frame stereo block at 44.1 kHz, Mix 100 %; `python3 tools/ci/instr_gate.py`, on CI) counts each engine; a rise of over 5 % fails unless explained and blessed.

| Engine | Before PR E (16 Nov) | With PRs E1 to E4 |
|---|---|---|
| Room | 171.9k | 145.9k (15 % fewer: chunks, the transform) |
| Plate | 186.7k | 155.8k (17 % fewer) |
| Spring | 185.6k | 138.0k (26 % fewer: four frames side by side) |
| Hall, Eco / Full | 249.0k / 723.6k | 267.5k / 791.2k |
| Cloud, Eco / Full | 254.4k / 908.7k | 275.0k / 991.8k |
| Echoes, Eco / Full | 169.8k / 282.5k | 183.7k / 313.2k |

PR E4 adds 7 to 11 % to every Cloud character and quality: each diffuser stage's loss is one multiply a frame (164 stages a frame in the Cloud character at Full), and the gains, level and fades glide per frame. Correct decay and a steady level were judged worth it.

**On the MPC** (an estimate until the next device round; [fx.md](fx.md) has the method): fitted from round 7's measurements, the Cloud engine at Eco takes about 7.4 to 11.4 % of a period (Plate 9.2, Room 8.5, Hall 9.8, Cloud 11.4, Echoes 7.4), at Light 10 to 17 % and at Full 11 to 51 %. One Eco reverb is about the whole 10 % effects budget, which is why reverbs belong on send returns (one reverb for many tracks), why returns can run a block late on the second core (`Mixer::set_pipelined_returns`), and why the cheaper Room, Plate and Spring matter: the old ones counted 155k, 139k and 206k instructions on ARMv7 under qemu (fx.md's table), and the rebuilt ones count 15 to 26 % fewer on the host; the device round measures them on the MPC. The lever left is fewer diffuser stages in Eco, a change of sound and so Jay's call, only if the device round says Eco is over budget.

## 9. Pitfalls and how the design avoids them

| Pitfall (research section 4) | Answer | Checked by |
|---|---|---|
| Metallic, ringing tails | Sixteen lines (or the Cloud engine's diffused loops), lengths off simple ratios, loss matched per line, slow wobble on long lines only, never on diffusers | R5 |
| Flutter or grain | Input diffusers; the Plate's dense onset; taps after the diffuser | R4 |
| Mud | Low cut on every engine; Damping; the Cloud's in-loop shelves | Presets' Low cut; listening |
| Wash, vocals pushed back | Pre-delay up to 100 to 500 ms; one slope (no front-loaded slap) | R3, R12 |
| Sibilance splash | Damping and Tone or High cut | Listening |
| Chorusy pitch wobble | Wobble only on long lines, depth chosen per voice | R9 |
| Level jumps between modes and presets; a Mix that turns the track down | Trims to the dry; input scaled by what the loop keeps; equal-power Mix | R6, R7, unit tests (a longer Decay not louder) |
| Decay not what the knob says | Loss solved per engine for the mid octaves | R1, unit tests at three rates |
| Mono collapse | Different output rows or seeds per side; Width | R8 |
| Tails cut off; clicks when controls move | Tail estimates for the slot; Size and Pre-delay glide or crossfade; loss filters glide | R11, the slot's tail tests |
| Pitch bends as Size moves | Crossfades instead of sliding lengths | `tests/cloud.rs`, R11 |
| Output depends on block size | A fixed chunk grid, feedback per frame | Bit-exact block-size tests |
| CPU and denormals | Chunked, stage-at-a-time loops; Eco; sleep on silence | The instruction gate; the idle gates |

## 10. History and next

- **Built:** the Cloud reverb (Cloud Seed port, F2; Eco, Light and Full; Eco's polyphase IIR half-bands, 11 Nov). The Room, Plate and Spring with the MPC coverage effects (14 Nov), and the Reverb family's modes (14 Nov).
- **The engines before PRs E1 to E3** (until 17 Nov):
  - *Room:* eight early reflections panned around the listener, then an eight-line feedback delay network of 19 to 48 ms at the default size, mixed by a Householder matrix and fed directly with no input diffusion, each line damped by a one-pole from Decay and Damping (Jot), with a slow modulation on a 16-frame grid. It summed the input to mono, sounded like a slap and a faint tail and took 220 to 314 ms to turn dense.
  - *Plate:* Dattorro's figure-eight tank (1997): four input diffusers, two tank halves (a modulated all-pass, a delay, damping and an all-pass) feeding each other, seven output taps a side, at his 29.761 kHz lengths scaled to the rate. Size moved its reads by whole frames, so it zipped.
  - *Spring:* the same chirp chains with the input summed to mono, Drip feeding each fresh chirp straight back outside the loss (so it changed Decay), Tone darkening every trip, an echo every Length plus the chirp's 9 ms, and a stretch fixed at one value.
- **16 Nov:** measuring built (fxlab quality, the ratchet); baseline 68 of 114 reverb targets.
- **PR B** (#423, 16 Nov): the Mix curve, equal power by default; the Room's wet signal +4 dB. 78 of 114.
- **PR E1** (17 Nov): the Room rebuilt, 87 of 114.
- **PR E2** (17 Nov): the Plate on the Room's network, 98 of 114.
- **PR E3** (17 Nov): the Spring, 102 of 114.
- **PR E4** (17 Nov): the Cloud engine's decay, level and crossfades (113 of 113, Echoes' R3 reported only).
- **Next** (plan section 7.2, P2, each its own pull request): measure R2 (Damping's ratio) and R13 (Freeze, spillover); **Ducking** (Amount, Release) on every reverb; **Pre-delay sync** (Free, 1/64, 1/32, 1/16); **Diffusion** on the Plate; a **bass multiplier** and crossover on the Room and Hall; **Freeze** on the Room and Plate; the wobble at Full quality; and the MPC's own numbers from the next device round.

## 11. Sources

- ISO 3382-1:2009, room acoustic parameters; the JNDs as tabulated by Christensen and Rindel, "The ISO 3382 parameters: can we simulate them? Can we measure them?", ISRA 2013: T20, T30 and EDT 5 %, C50 and C80 1 dB.
- M. R. Schroeder, "Natural sounding artificial reverberation", JAES 10(3), 1962; J. A. Moorer, "About this reverberation business", Computer Music Journal 3(2), 1979.
- J. Stautner and M. Puckette, "Designing multi-channel reverberators", Computer Music Journal 6(1), 1982.
- J.-M. Jot and A. Chaigne, "Digital delay networks for designing artificial reverberators", AES 90th Convention, 1991.
- S. Schlecht and E. Habets, "Accurate reverberation time control in feedback delay networks", DAFx 2017.
- V. Välimäki, J. Parker, L. Savioja, J. O. Smith and J. Abel, "Fifty years of artificial reverberation", IEEE TASLP 20(5), 2012.
- J. Abel and P. Huang, "A simple, robust measure of reverberation echo density", AES 121st Convention, 2006.
- A. Lindau, L. Kosanke and S. Weinzierl, "Perceptual evaluation of model- and signal-based predictors of the mixing time in binaural room impulse responses", JAES 60(11), 2012.
- J. Dattorro, "Effect design, part 1: reverberator and other filters", JAES 45(9), 1997 (the old Plate).
- V. Välimäki, J. Parker and J. Abel, "Parametric spring reverberation effect", JAES 58(7/8), 2010 (the Spring).
- Ghost Note Audio, [Cloud Seed](https://github.com/GhostNoteAudio/CloudSeedCore) (MIT), the Cloud engine's structure and seeds.
- ValhallaDSP, "Reverbs: diffusion, allpass delays, and metallic artifacts" (2011) and "Getting started with reverb design, part 2" (2021).
- B. Fazenda, M. Stephenson and A. Goldberg, "Perceptual thresholds for the effects of room modes as a function of modal decay", JASA, 2015.
- ITU-R BS.1770, loudness (the level measures).
- Usage: Sound On Sound, "Choosing the right reverb"; the Abbey Road reverb EQ (Soundfly).
