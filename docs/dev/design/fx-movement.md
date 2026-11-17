<!--
covers: rust/mpfree-fx/src/motion.rs rust/mpfree-fx/src/shift.rs
checked: 2026-11-18 b4fe3b81
-->
# Movement: tremolo, auto-pan, rotary, ring mod, frequency shifter and auto wah

**Status:** built (F2 and the MPC coverage round); FX quality round 5 is under way. Its research, targets and measuring are done (101 of 171 movement targets met before any fix, at 44.1 kHz). PR W (17 Nov) rebuilds the LFO they share and crossfades the tremolo's Mode: 136 of 171 met, every LFO, tremolo and auto-pan row among them. PR 2 (17 Nov) rebuilds the rotary's motion, beam, level and drive: 148 of 171, every rotary row among them. The auto wah, and the ring mod with the frequency shifter come next, a PR each. Below, "before PR W" and "before PR 2" describe what was there before.

This page is the design of MPFree's movement effects as one effect type: what they are for, the ideas underneath them, the research behind ours, which algorithms each uses and why, how quality and cost are measured, and how the defaults and presets were chosen. The parameter-by-parameter summary lives in [fx.md](fx.md); the measured targets and their history in [fx-quality.md](fx-quality.md#round-5-movement-pitch-and-voice). The research notes, plan and baseline probe are in jferments/mpfree-claude `fx/quality/` (research-movement.md, plan-5.md, baseline-r5-2026-11-17.md, tools/r5_baseline.rs).

| Effect | Node id | Code | One line |
|---|---|---|---|
| Tremolo | `mpfree.fx.tremolo` | [`motion.rs`](../../../rust/mpfree-fx/src/motion.rs) | The level moved by an LFO; Harmonic mode moves the lows and highs in turn |
| Auto-pan | `mpfree.fx.autopan` | `motion.rs` | The balance moved by the same LFO, at constant power |
| Rotary | `mpfree.fx.rotary` | `motion.rs` | A Leslie: a horn and a drum spinning past two microphones, ramping between Slow and Fast |
| Ring mod | `mpfree.fx.ringmod` | `motion.rs` | The sound times a carrier, swept by the LFO |
| Frequency shifter | `mpfree.fx.freqshift` | [`shift.rs`](../../../rust/mpfree-fx/src/shift.rs) | Every partial moved by the same number of hertz |
| Auto wah | `mpfree.fx.autowah` | [`wah.rs`](../../../rust/mpfree-fx/src/wah.rs) | A resonant filter moved by the playing, an LFO or a pedal |

The LFO they share (`kit::Lfo`, in [`kit.rs`](../../../rust/mpfree-fx/src/kit.rs)) also moves the talk box's vowel. The chorus, flanger and phaser have their own ([fx-modulation.md](fx-modulation.md)).

## 1. What they are for

All six move something over time: a level, a position, a pitch or a filter. What moves, and how fast, makes the difference (research-movement.md section 1).

| Effect | What moves | Typical settings | Jobs |
|---|---|---|---|
| Tremolo | The level | 2 to 10 Hz or a note value; Depth from a shimmer to a full chop | The amp wobble on guitar and keys, a synced chop on pads (the "trance gate"), a slow swell; Harmonic is the early-1960s Fender swirl |
| Auto-pan | The balance | 0.1 to 4 Hz or a note value; Depth to hard left and right | Movement in a mix, ping-ponged hats and percussion, a slow drift on a pad |
| Rotary | A horn (highs) and a drum (lows) spinning past microphones | Slow (chorale) and Fast (tremolo), and the ramp between them | Organ above all; guitar, Rhodes, vocals. The ramp up and down is played |
| Ring mod | The sound times a carrier | 20 Hz to a few kHz (below about 20 Hz it is a tremolo) | Bells and metallic tones, robot voices (about 30 Hz), sci-fi sweeps, inharmonic drums |
| Frequency shifter | Every partial, by the same hertz | A few tenths of a hertz (barber-pole phasing) to hundreds (metallic, detuned) | Barber poles, metallic drums, spiralling feedback, width a few hertz up on one side and down on the other |
| Auto wah | A resonant band-pass | About 400 Hz to 2 kHz, quick attack, 100 to 300 ms release | Funk guitar and clavinet, bass, synth squelch; Pedal for a fixed "cocked wah" |

The familiar units set what musicians expect: Fender's brown-panel harmonic tremolo (1960 to 1963), the Leslie 122 and 147 (horn about 48 and 400 rpm, drum about 40 and 342 rpm; the horn reaches full speed in about 1.5 s and the drum in about 6 s), Moog's MF-102 ring modulator, Bode's and Moog's frequency shifters, and the Mu-Tron III and the Cry Baby for the wah.

## 2. Principles

**An LFO's jumps are clicks.** A gain or a pan that changes in one frame puts a step into the sound, and a step's spectrum falls only 6 dB an octave: a clear click above 6 kHz on any tone. A ramp that resets, a square's edge and a random value's change are all jumps unless they take a few milliseconds. The click measure hears 1 ms of a 997 Hz tone above 6 kHz; a 4 ms raised-cosine edge reads -86 to -102 dB on it, a one-frame reset -18 dB (PR W's numbers).

**An edge should be a time, not a share of the cycle.** A square made by clipping a sine has edges a fixed share of its period long: crisp at 20 Hz, an 86 ms fade at 0.5 Hz. A chop should be a chop at any rate, so the edge is a fixed time (4 ms here, 10 to 90 % in 2.36 ms), shortened only where the cycle is too short for it (an eighth of a cycle at most).

**Synced means on the song, all the time.** An LFO that only lines up with the bar when the transport starts drifts off it when a loop wraps or the song jumps: a 3.5-bar loop leaves a one-bar LFO half a cycle off. Following the song position all the time keeps it on the bar; the jump itself is then a jump in the LFO's output, so it glides (round 3 fixed the same for the chorus, flanger and phaser).

**A tremolo's gain only dips.** `g = 1 - d (1 - m) / 2` for an LFO value `m` from -1 to 1 and Depth `d`: the level swings between 1 and `1 - d`, so Depth 50 % is a 6 dB swing and 100 % a full cut, and the tremolo goes in quieter by what its depth takes, like the amps. Harmonic mode splits the sound with gentle slopes into two bands that sum back to the input and moves them in opposite phase, so the tone swirls while the whole moves little.

**A pan keeps its power.** Left `cos θ`, right `sin θ` (θ from 0 to a quarter cycle) keeps `L² + R²` constant, so the sound keeps its loudness as it travels; scaled by √2, both sides are at unity in the middle. On a stereo source this is a balance: each side fades at the other extreme, as Ableton's Auto Pan does.

**A Leslie is Doppler, level and a ramp.** A horn mouth on a radius of about 17 cm at 400 rpm moves at about 7 m/s, about ±36 cents of Doppler; its level swings as it points at and away from the microphone, more for the highs (the horn's beam narrows with frequency); the drum does the same in the lows, more slowly and gently. The rotors have inertia, the light horn more than the heavy drum, and slow down more slowly than they speed up.

**A ring mod's products and a shifter's image.** A sine carrier `c` times a sine input `x` gives only `x + c` and `x - c`; a square carrier adds every odd harmonic's products, a buzzier sound, and those above Nyquist fold back unless the carrier is band-limited. A frequency shifter keeps only one side: the analytic signal (the input and its 90° copy) times a complex phasor. How well the 90° network keeps its phase across the band sets how far down the wrong-way image is.

**A narrower band-pass passes less.** A band-pass's power on a full-band sound falls with its bandwidth, about `10 log10` of the ratio, so a resonant wah is quieter unless it is compensated.

## 3. Research

research-movement.md has the detail and the sources. In short:

- **Users talk about the rotary's ramp** more than anything else, and Logic's Rotor Cabinet offers a brake, an acceleration control, "Inv" (the drum fast while the horn is slow) and "910" (the drum held slow). Smith, Serafin, Abel and Berners model the horn and the baffle as separate rotating sources with angle-dependent filtering and different shapes for speeding up and slowing down.
- **Tremolos** offer sine, triangle, square, ramps and random, sync with dotted and triplet values, Depth, a stereo phase (Width) and Harmonic mode; some add a smoothing control for the square and an envelope on the depth.
- **Frequency shifters** (Ableton's Shifter, Valhalla FreqEcho, Bitwig's) offer up, down, both, stereo and feedback for spirals. An 8-pole 90° network keeps about 44 dB of image rejection; 12 to 16 poles reach 70 to 90 dB over the band.
- **Wahs:** Faust's crybaby (from CCRMA's measurement of a Dunlop Cry Baby) runs from about 450 Hz at the heel to 2.2 kHz at the toe, with a Q falling from about 8 to 2 as it opens.

## 4. The engines

### The shared LFO

`kit::Lfo` is free in hertz or synced to the tempo (1/32 to 4 bars, dotted and triplets), with six shapes (Sine, Triangle, Square, Ramp up, Ramp down, Random). Its phase is a double, so a long free run never drifts or rounds. Since PR W:

- **Edges and returns:** a Square's edges are a quarter of a sine across 4 ms (`EDGE_MS`), at most an eighth of a cycle (`EDGE_CYCLES`); a Ramp's return is a raised cosine from the end it reached back onto its line over the same time, starting at the wrap, so the ramp still lands on the beat. Before PR W the Square was a sine times 6, clipped (edges of 86 ms at 0.5 Hz), and the Ramps reset in one frame.
- **Random:** a raised-cosine glide over each cycle from one random value to the next, as before; the values are worked out a cycle ahead, so a Width offset (the tremolo's right side, up to half a cycle later) glides through the same values instead of jumping where its own phase wraps. The phase is read as the largest float under 1 at most, so a phase just short of the wrap never rounds onto the next cycle's start (before PR W it did at some rates, a one-frame spike once a cycle at 48 kHz).
- **Sync:** synced and playing, the LFO checks the song position at the start of every block and after every control change, and lands on the bar when the transport starts. When the two differ by more than a millionth of a cycle (a loop wrap, a locate, a Sync change), the phase goes where the song says and the output glides there. Rounding between the tempo and the frame count is left alone, and a new phase is put on a grid of 2^-40 of a cycle, so the output is the same, bit for bit, at any block size.
- **Jumps:** a Shape change, a Sync change, a loop wrap or a locate (anything that moves the phase more than a millionth of a cycle) puts the LFO where it should be at once; its output glides there over 20 ms (`JUMP_MS`) as an offset that fades along a raised cosine. The phase is right at once, so a synced LFO is back on the song as soon as the glide ends. Nothing glides on the first frame after a reset.
- **Reading:** `step(offsets)` steps a frame and gives the value at one or two offsets; `tick` steps without reading and `read` reads, for an effect that needs the value only every few frames. The glide, the wrap and the first frame after a reset share one counter, so a frame with none of them costs one compare.

### Tremolo

The gain above, from the LFO, per side; Width moves the right side's LFO up to half a cycle later (a stereo shimmer). **Harmonic** mode splits at 700 Hz with a 12 dB low-pass (`Svf`, Q 0.707) and the input minus it, so the bands always sum back to the input, and moves them in opposite phase. A Mode change crossfades the high band's gain between Volume's and Harmonic's over 20 ms (PR W; before, the switch was immediate, -23 dB on the click measure), the split starting from rest when it has been idle. Depth and Width glide over 20 ms. In Volume mode with Width at 0, the default, one LFO value serves both sides.

### Auto-pan

The constant-power law above, at `Centre + Depth × m`, clamped to the extremes; sine and cosine come from one polynomial (`kit::sine`). Depth and Centre glide over 20 ms.

### Rotary

After J. O. Smith's rotating horn and the daisy-leslie project: a Linkwitz-Riley crossover at 800 Hz, then the horn and the drum each a Doppler delay (cubic reads) whose length follows the rotor's angle, and a level following the rotor past two microphones. Speed moves between Stop, Slow (horn 0.8 Hz, drum 0.67) and Fast (6.7 and 5.7 Hz). Balance sets horn against drum, Width the microphones' spread.

- **The ramp:** each rotor follows the speed through a one-pole. Ramp (200 ms to 8 s) is the time the drum takes from Slow to within 10 % of Fast: 2.18 time constants, the gap from 5.03 Hz down to 0.57 (`RAMP_TAUS`). The light horn takes about a quarter of it (1.1 s at the default 4.5 s). Both coast down a quarter more slowly than the motor speeds them up (`COAST`), as Smith, Serafin, Abel and Berners describe.
- **Opposite rotation:** the drum turns the other way from the horn, as a Leslie's do, so the microphone that hears the horn first hears the drum last.
- **The treble beam:** a horn's beam narrows with frequency. Facing away from a microphone, the horn is heard through a one-pole low-pass at 2 kHz (`BEAM_HZ`, one a microphone), crossfaded to the whole horn as it turns to face it, so the highs swirl more than the mids.
- **Level:** the horn and the drum are 1.07 and 1.28 times up at the microphones, so the cabinet goes in at the dry's level, swings and all.
- **Drive:** as round 2's: soft saturation averaged by first-order ADAA, 2x oversampled through Drive's Eco half-bands, its level held by the curve's ceiling (`undrive`), so noise at -18 dBFS stays within 0.3 dB from Drive 0 to 100. It runs only while Drive is up and fades in and out; the plain path is held the same 4 frames late, so turning Drive on or off never moves the sound in time.
- **Angles:** each rotor's angle at each microphone comes from the rotor's and the spread's sines by the angle sums, six sines a frame instead of eight, which pays for the beam with room to spare.

Measured before PR 2 (round 5's baseline): the speeds within 0.1 % and the Doppler ±36.8 and ±8.1 cents; the horn reached Fast in 0.82 s and the drum in 3.28 s, and both slowed down at the same pace; it went in 3.2 LU quieter; the horn had no beam (its 6 kHz swing 1.7 dB less than its 1.5 kHz one) and both rotors turned the same way; Drive folded back at -30.8 dB (Drive 50) and -13.3 dB (100). After it every rotary target is met: the horn reaches Fast in 1.14 s and the drum in 4.66 s, and they take 1.42 and 1.25 times as long to slow down; the Doppler ±37.6 and ±8.3 cents; within 0.3 LU of the dry on pink and hits, and every preset within 0.2; a 5.3 dB beam; opposite rotation; Drive's fold-back at -96.3 dB (Drive 50) and -55.6 dB (100).

### Ring mod

The input times a carrier from 20 Hz to 5 kHz (capped at 0.45 of the rate): Sine, or Triangle and Square band-limited with PolyBLAMP and PolyBLEP. Sweep moves the carrier up to four octaves with the LFO's sine (read only while Sweep is up). Freq and Sweep glide in octaves over 30 ms, Mix over 20 ms. Measured: the sine leaves spurs at -62.2 dB; a triangle aliases at -62.5 dB and a square at -36.8; the waves are 5.0 LU apart; a Wave change spikes 5.8 dB. PR 4's (plan-5 section 8).

### Frequency shifter

Single-sideband shifting by 0.01 Hz to 5 kHz: Olli Niemitalo's eighth-order pair of all-pass chains, 90° apart within 0.7° from 20 Hz to 22 kHz, makes the analytic signal, and a phasor (its phase in double precision, so 0.01 Hz still turns smoothly) shifts it. Modes as the AIR one: Up, Down, Up and down, Stereo (left up, right down). Feedback to 95 % through a soft clip, the output scaled by √(1 - g²), since each pass lands on new frequencies and the passes add in power. No latency. Measured: the image is -44.4 dB, the spurs -59.8 to -60.8, and a Mode change clicks at -13.4 dB. PR 4's.

### Auto wah

One Simper state-variable filter swept evenly in pitch between Low and High (100 Hz to 6 kHz) by the sound's level (a follower with Sensitivity, Attack and Release), by the LFO (its sine, read when the filter retunes), or by hand (Source Pedal: Center is the pedal). A negative Depth sweeps down. Shapes: Wah (the band-pass with the low end leaking through at about -9 dB, as a pedal's does), Band-pass (level-normalised), Low-pass and High-pass. It retunes every 16 frames from a smoothed position. Measured: the peak within 0.6 % at Resonance 60 and 100, 3 % at 0; 5.7 LU quieter at the defaults; Band-pass 3.6 LU louder at Resonance 0 than at 60; Filter clicks at -8.4 dB and a fast Resonance move at -42.3. PR 3's (plan-5 section 7).

## 5. Controls

| Effect | Controls (in id order) |
|---|---|
| Tremolo | Rate (0.05 to 20 Hz), Sync (Free, 1/32 to 4 bars), Shape, Depth, Width, Mode (Volume, Harmonic) |
| Auto-pan | Rate, Sync, Shape, Depth, Centre |
| Rotary | Speed (Stop, Slow, Fast), Ramp, Balance, Width, Drive, Mix |
| Ring mod | Freq, Wave (Sine, Triangle, Square), Sweep (octaves), LFO rate, LFO sync, Mix |
| Frequency shifter | Shift, Mode (Up, Down, Up and down, Stereo), Feedback, Mix |
| Auto wah | Source (Envelope, LFO, Pedal), Center, Depth, Sensitivity, Attack, Release, Rate, Sync, Low, High, Resonance, Filter, Mix |

PR W and PR 2 change no control and no id, so saved projects open as they were; they sound as described above (a saved rotary now goes in at the dry's level, its drum turns the other way, its horn beams and its ramp takes the Ramp's time).

## 6. Defaults and presets

- **Tremolo** at 4 Hz, Sine, Depth 50 %: the amp wobble. Presets: Amp wobble, Eighth chop (1/8, Square, Depth 100: the trance gate, now crisp at every tempo), Stereo shimmer (Width 100), Harmonic vibe, Slow swell (1 bar, Triangle).
- **Auto-pan** at 1/4, Sine, Depth 70 %. Presets: Gentle drift (2 bars), Hard ping (1/8 Square), Random space (1/16, Random).
- **Rotary** at Slow with a 4.5 s Ramp. Presets: Chorale, Tremolo fast, Gritty organ, Quick ramp.
- **Ring mod** at 440 Hz Sine, no Sweep. Presets: Robot voice, Metal bell, Sci-fi sweep, Square buzz.
- **Frequency shifter** 100 Hz up. Presets: Barber pole, Metal drums, Falling spiral, Wide shimmer, Robot.
- **Auto wah** on the envelope, Wah shape. Presets: Funk quack, Slow bloom, Down wah, Eighth sweep, Pedal.

## 7. Measuring quality

fxlab's `movement` class (`quality/round5.rs`, ids W and Z) measures them, and the ratchet holds every number ([fx-quality.md](fx-quality.md#round-5-movement-pitch-and-voice) has the targets and the history):

- **The LFO as heard** (W1, W2): the tremolo's gain frame by frame on a steady input at Depth 100, every shape at 0.5, 4 and 20 Hz, at 44.1 and 48 kHz: the largest change in one frame (≤ 0.05 of the range), the click measure (≤ -60 dB) and a Square's 10 to 90 % rise (1 to 4 ms).
- **Sync after a wrap** (W3): the song looping from 3.5 bars to the start, the LFO at 1 bar: its phase against the song before the wrap and from 50 ms after it (within 0.01 of a cycle).
- **Clicks on every control** (W4, W7, and the rest): round 3's M13, each control swept end to end in 50 ms and back under a 997 Hz tone (≤ -60 dB).
- **Depth, Harmonic, power** (W5, W6, W8): the swing against `20 log10(1/(1 - d))`; Harmonic's whole and its 3 kHz tone on two steady tones; the auto-pan's L² + R².
- **The rotary** (W9 to W14), **the auto wah** (W15 to W18), **the ring mod and shifter** (Z1 to Z6): speeds, Doppler and ramps from the analytic signal of tones through the rotors; level, beam, direction and drive; the wah's peak and level; products, images, spurs and feedback.
- Unit tests in `kit.rs`: every shape in range at any offset; no frame-to-frame step over 0.05 of the range at 0.5, 4, 19.7 and 20 Hz, at 44.1 and 48 kHz, at offsets 0, a quarter and a half; a Square's edge 1 to 4 ms at 0.5, 4 and 20 Hz; a synced LFO after a loop wrap equal to a fresh one within 20 ms and gliding there; a Shape change between any two shapes gliding.

## 8. Cost

The instruction gate, rustc 1.97.0, a 128-frame block at 44.1 kHz, at the defaults:

| Case | Before round 5's fixes | Now |
|---|---|---|
| Tremolo | 16.7k | 11.3k |
| Auto-pan | 22.0k | 21.3k |
| Ring mod | 24.6k | 20.3k |
| Auto wah | 28.5k | 25.5k |
| Frequency shifter | 55.3k | 55.3k |
| Rotary | 115.7k | 108.2k |

PR W's LFO does more and costs less: its glide and wrap are out of line, the Sine is tested first, the tremolo works out one value a frame in its usual case, and the auto wah and talk box read the LFO only when they retune. The first version, with the glide in line and the shape matched for every value, cost the tremolo 45 % more.

PR 2's rotary does more for 6.5 % less: the angle sums take two sines a frame off, more than the beam's two one-poles and the drive's delay cost.

## 9. Pitfalls and how the design avoids them

- **A reset in one frame** (the Ramps before PR W): a click every cycle. Every jump in a shape takes 4 ms.
- **An edge that scales with the period** (the clipped-sine Square): a fade at slow rates. The edge is a time.
- **A float phase that rounds up** at the end of a cycle: one frame of the next cycle's start (Random's spike). The phase is read short of 1.
- **An offset that wraps on its own** (Random with Width): the right side jumped where its phase wrapped. Random's values are worked out a cycle ahead.
- **Sync only at the transport's start**: off the bar after a loop wraps. The phase follows the song all the time, and the jump glides.
- **A mode or shape switched in one frame**: a click. Both glide over 20 ms.
- **Rotors that turn together, a horn with no beam** (the rotary before PR 2): a flatter swirl. The drum turns the other way, and the horn's highs are low-passed while it faces away.
- **A ramp read one rotation at a time** (plan-5's 4.5 s for the drum was 3.28 s at every moment): the ramp is set from the speed at every moment, and slowing down is slower than speeding up.
- **Saturation at the base rate** (the rotary's Drive before PR 2): fold-back at -13 dB. It runs 2x oversampled with ADAA, and the plain path waits as long, so switching never shifts the sound.
- **The ring mod's aliasing, the shifter's image, the wah's level:** measured, and the subjects of the next PRs (plan-5 sections 7 and 8).

## 10. History and next

- F2 and the MPC coverage round: the tremolo, auto-pan, ring mod and rotary (`motion.rs`), the frequency shifter (`shift.rs`), the auto wah (`wah.rs`).
- FX quality round 5 (17 Nov): research, baseline and plan (mpfree-claude #211, plan-5.md); the measuring PR (#510): 101 of 171 movement targets met.
- PR W (17 Nov): the shared LFO rebuilt and the tremolo's Mode crossfaded: 136 of 171.
- PR 2 (17 Nov): the rotary's ramp, coast, opposite rotation, treble beam, level and oversampled drive: 148 of 171.
- Next: PR 3 the auto wah (level, retuning, the Filter switch), PR 4 the ring mod and frequency shifter (the square's aliasing, the waves' level, a better 90° network, the Mode switch).

## 11. Sources

- research-movement.md (jferments/mpfree-claude `fx/quality/`) and its sources: the Musicplayer forum on Leslie rotor speeds; J. Smith, S. Serafin, J. Abel and D. Berners, "Doppler simulation and the Leslie", DAFx 2002, and Stanford's discrete-time Leslie emulation; Apple's Logic Pro Rotor Cabinet; CCRMA's Cry Baby measurement; O. Niemitalo's Hilbert transform all-pass pairs; the Moog MF-102 manual; Fender's brown-panel harmonic tremolo schematics.
