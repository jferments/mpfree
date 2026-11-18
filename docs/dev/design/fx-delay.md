<!--
covers: rust/mpfree-fx/src/delay.rs rust/mpfree-fx/src/echoes.rs rust/mpfree-fx/src/kit.rs
checked: 2026-11-17 8ac941f3
-->
# Delays: Digital, Tape and Multi-tap

**Status:** built; FX quality PRs B (16 Nov, the Mix curve and the echo tails) and C (16 Nov, flat reads, oversampled loop saturation, the Tape's colour at the record head) are in. 74 of 80 measured targets met. What is left is under [History and next](#10-history-and-next).

This page is the design of MPFree's three delays as one effect type: what a delay is for, the ideas underneath it, the research behind ours, which algorithms each engine uses and why, how quality and cost are measured, and how the defaults and presets were chosen. The parameter-by-parameter summary lives in [fx.md](fx.md) (the Delay, Tape delay and Multi-tap entries); the measured targets and their history in [fx-quality.md](fx-quality.md#delay). The research notes, plan and baselines are in jferments/mpfree-claude `fx/quality/` (research-delay.md, plan.md section 5, baseline-2026-11-16.md).

| Engine | Node id | Code | One line |
|---|---|---|---|
| **Digital** (the Delay family's default mode) | `mpfree.fx.delay` | [`delay.rs`](../../../rust/mpfree-fx/src/delay.rs) | A clean stereo echo with a filtered, saturated loop, ping-pong and freeze |
| **Tape** | `mpfree.fx.tape_delay` | [`echoes.rs`](../../../rust/mpfree-fx/src/echoes.rs) | A three-head tape echo: wow, flutter, the tape's colour on every echo, a loop that can run away |
| **Multi-tap** | `mpfree.fx.multitap` | [`echoes.rs`](../../../rust/mpfree-fx/src/echoes.rs) | Four taps on the sixteenth grid, each with level and pan, the last feeding back |

The shared parts are in [`kit.rs`](../../../rust/mpfree-fx/src/kit.rs): `Line` (the delay line), `FlatRead` (the fractional read), `Os2` (twice-the-rate saturation), `Blend` and `mix_gains` (the Mix curve).

## 1. What a delay is for

A delay plays the sound back later, once or many times. Musicians reach for it for five jobs, and each wants something different (research-delay.md section 1, from common practice and the manuals of the delays people know):

| Job | Typical time | Feedback | Mix | What matters most |
|---|---|---|---|---|
| **Slapback** (rockabilly vocals, guitars, snares) | 70 to 150 ms | 0 to 15 % | 15 to 30 % | One clean, slightly dark repeat |
| **Doubling and width** | 10 to 40 ms | 0 % | 20 to 50 % | Holds up in mono; doesn't comb |
| **Rhythmic echo** (1/8, dotted 1/8, 1/4) | Synced | 20 to 50 % | 10 to 30 %, or 100 % on a return | Exactly on the grid; repeats that sit behind the dry |
| **Dub and throws** | Synced, often dotted | 60 to over 100 % | 100 % on a return | Feedback past 100 % that howls but never blows up; filter and time played live |
| **Ambient and texture** | 300 ms to 2 s | 50 to 90 % | 30 to 60 % | Repeats that blur; freeze |

Digital covers rhythmic echo, slapback and throws; Tape covers slapback, dub and anything that should sound old; Multi-tap covers rhythms a single time can't make (a dotted bounce, a triplet scatter, a pattern across the stereo field). Doubling is reachable on Digital in Free time but has no dedicated controls yet (see [What is left](#10-history-and-next)).

## 2. Principles

**The loop.** A feedback delay writes `input + feedback × output` into a line and reads it back `Time` later. Each pass round the loop multiplies an echo by the loop's gain: after four passes an echo is 24 dB down at Feedback 50 %, and about 36 dB down at 35 % (Digital's default). Everything in the loop (filters, saturation, the interpolation of a fractional read) is applied once per pass, so its effect compounds: a loss of 0.7 dB a repeat at 10 kHz is 6 dB after eight repeats. Design rule: anything in the loop must be either intended (Tone, Low cut, the tape's colour) or exactly transparent.

**Hearing a repeat.** Two copies of a sound closer than about 30 to 40 ms fuse into one, heard from the earlier copy's side (the precedence, or Haas, effect: Wallach et al. 1949, Litovsky et al. 1999). Above that the copy is an echo. That boundary is why doubling and slapback are different jobs.

**Fractional delays.** A synced time is rarely a whole number of frames (a dotted 1/8 at 120 BPM is 16,537.5 frames at 44.1 kHz). Reading between samples needs interpolation. Polynomial interpolators (linear, cubic Hermite, Lagrange) are low-pass filters whose loss depends on the fraction: none at a whole frame, worst at half a frame (Laakso et al. 1996). Inside a loop that loss compounds, so the brightness of the repeats would depend on the tempo. A first-order all-pass (Thiran) read has a flat magnitude at every frequency and an exact delay at low frequencies; it rings briefly if its coefficient moves fast, so it suits a still time and not a glide.

**Saturation in the loop.** A soft clipper in the loop keeps feedback over 100 % bounded: each pass grows the echo until the clipper's ceiling holds it, so a throw howls instead of exploding. But a clipper makes harmonics, and harmonics above Nyquist fold back as inharmonic aliases, which the loop then plays again on every repeat. The saturator must be antialiased.

**Tape.** A tape echo records onto a moving loop of tape and plays it back from heads further along, so every echo, the first included, has been through the record and playback chain once: some top-end loss, a low-frequency "head bump", soft saturation. Its speed wanders slowly (wow, under about 4 Hz) and quickly (flutter, about 4 to 100 Hz); professional machines reach about 0.02 % weighted, worn ones well over 1 % (IEC 386 / AES6 measurement). Changing the time changes the tape speed, so the pitch slides.

**The mix law.** The echo is a different moment of the music, so it is uncorrelated with the dry: a linear crossfade loses 3 dB of power half way. An equal-power curve (cosine for the dry, sine for the wet) keeps the level as Mix turns. Jay picked equal power as the default for every delay and reverb on 16 Nov, with Dry first and Linear as choices (fx.md, [the Mix curve](fx.md#the-delays-and-reverbs-mix-curve)).

## 3. Research

The research note (mpfree-claude `fx/quality/research-delay.md`) went through the manuals, reviews and papers listed under [Sources](#11-sources) to answer three questions: what "good" means in numbers, the settings people use, and the features they look for. The main findings that shaped the engines:

- **What separates a good delay from a cheap one** is mostly invisible on the panel: repeats that land on the grid, repeats that keep their top end unless Tone says otherwise, a loop that can run away safely, saturation that doesn't alias, no zips or clicks when a knob moves, and a Mix knob that doesn't make the track quieter. Each became a measured target (section 7).
- **Features people expect first** (from Soundtoys EchoBoy, ValhallaDelay, Ableton's Delay and Echo, Elektron's Digitakt delay): synced time with dotted and triplet values, feedback with a filter on the repeats, ping-pong, a tape flavour, something for throws (freeze, or feedback over 100 %). MPFree has those. The gaps that matter most: ducking, a low cut on Digital, a choice of how a time change sounds (Ableton's Repitch, Fade and Jump), modulation on the clean delay, separate left and right times.
- **Feedback filtering** is standard: a high-pass of 100 to 400 Hz so repeats don't pile up bass and a low-pass of 2 to 8 kHz so they sit behind the dry (the Digitakt puts both in its loop; EchoBoy's per-band decay works on each repeat).
- **Tape models** range from a filter and a clipper to physical models of the hysteresis (Arnardottir, Abel and Smith 2008 for the Echoplex; Chowdhury 2019). A hysteresis model needs an iterative solver per sample and was not worth its cost on the MPC's ARM for an echo whose job is colour, not fidelity to one machine.
- **The MPC's own delays** (AIR) have not been measured yet: fxlab can read FXProbe captures of them (fxlab.md), but the capture has not run on the device. No claim on this page compares MPFree's delays with AIR's.

The baseline measured on 16 Nov before any fix found the gaps the PRs then closed: repeats losing 0.74 dB a repeat at 10 kHz and 3.5 dB at 15 kHz at the default dotted 1/8 at 120 BPM (a Hermite read half a frame off); the Tape's first echo with none of the tape's colour; Mix dipping 2 to 3 LU half way; the Tape's saturation aliasing at -46 dB.

## 4. The engines

### Digital

```
in ──┬─────────────────────────────────────────────────────┬── Blend ── out
     │ (ping-pong: L+R into the left line, the lines cross) │ (dry, wet)
     └─► + ─► line ─► FlatRead ─► High-pass 60 Hz ─► Tone ─► Os2(soft) ─┐
         ▲                         (the read is the wet)                │
         └──────────────── × Feedback (Freeze: 1, raw) ◄────────────────┘
```

- **Time:** Free (1 ms to 4 s, log) or synced to one of twelve values from 1/32 to 1 bar, straight, dotted and triplet. The time in frames glides (a 60 ms one-pole), so a change slides in pitch like tape instead of clicking.
- **The read:** `kit::FlatRead`. While the time holds still it is a first-order all-pass on the fraction of a frame: flat to 20 kHz at any fraction, so the repeats are equally bright at every tempo. While the time glides it is the cubic (Catmull-Rom), which follows a moving point without ringing. Each time the delay starts or stops moving the two cross-fade over 3 ms, and the all-pass's state is the read's own last output, so it picks up where the cubic left off.
- **The loop:** a fixed 60 Hz high-pass (12 dB per octave, Butterworth) so repeats don't build up sub-bass, the Tone low-pass (500 Hz to 20 kHz, Butterworth), then the saturator. Saturation drives `soft` (a rational curve, `x (27 + x²) / (27 + 9x²)`, smooth and exactly ±1 from |x| = 3) by up to 18 dB and divides the drive back out, so a quiet repeat passes at unity and only loud ones are shaped. Below 100 % Feedback is therefore the loop's gain; above it (to 110 %) the echoes grow into the clipper's ceiling.
- **Oversampled saturation:** the saturator runs at twice the rate between polyphase IIR half-bands (`kit::Os2`, three first-order all-pass sections each way). Its harmonics above the base Nyquist are filtered off instead of folding back, so a 5 kHz full-scale sine's worst alias is under -87 dB on every preset (it was -50 dB at Saturation 60 % without). Up then down is one all-pass, so the loop keeps its whole band, about 1.3 frames late at low frequencies.
- **Why oversampling and not ADAA:** first-order antiderivative antialiasing was the plan's first idea (cheaper, no latency). But on a quiet signal first-order ADAA is a two-point average, a low-pass that loses about 2.4 dB at 10 kHz and 6.3 dB at 15 kHz at 44.1 kHz, and in a loop that is exactly the hidden, compounding loss the flat read removes. The half-band pair is flat in level, so it costs more but loses nothing.
- **Ping-pong:** the input's mono sum goes into the left line only and each line feeds the other, so the first repeat is on the left and the next on the right (fxlab D10: each repeat 60 dB louder on its side).
- **Freeze:** the input fades out and the loop's gain goes to 1 with the filters and saturation bypassed (glided over 30 ms), so the buffer plays as a loop for ever. Its tail is endless until Freeze is off.
- **Tail:** the time plus passes until Feedback has lost 60 dB, capped at 30 s; Feedback at 100 % or more never dies away, so the slot keeps playing until it comes down.

### Tape

```
in (L+R)/2 ─► + ─► Tone ─► Low cut ─► head bump ─► Os2(saturation, ceiling) ─► tape
              ▲                                                               │
              └── × Feedback ◄── mean of the heads ◄── FlatRead × 1 to 3 heads ◄┘
                                     │ (Spread pans them)
                                     └──► × TAPE_TRIM (+2.5 dB) ── Blend ── out
```

- **The record head carries the colour.** The input and the feedback go onto the tape together through one pass of the tape's tone: Tone (a 12 dB per octave low-pass at Q 0.6, 800 Hz to 16 kHz), Low cut (a 12 dB per octave high-pass at Q 0.6, 20 to 800 Hz) and the heads' bump (+2 dB at 110 Hz, a bell at Q 0.8), then the saturation. So the first echo has been round the tape once and each repeat once more, as on a real machine (fxlab D8: the first echo now carries half the second's darkening at 10 kHz; it had none). Exactly what the Tone card draws is what a pass does.
- **Saturation that never adds gain:** `lerp(x, tanh(drive·x)/drive, s)` with drive 1 + 4s, then a ceiling that is the input up to full scale and bends smoothly toward 1.25 above it, both at twice the rate (`kit::Os2`). Because the curve's slope is at most 1, the loop's gain is Feedback times the tone's gain, so Feedback means what it says and the Dub echo preset at 80 % dies away. Over 100 % (the range goes to 150 %) the echoes grow about 2 dB a pass until the ceiling holds them: Runaway (130 %) and the Dub echo XY pad (up to 130 %) live there.
- **Heads:** one, two or three playback heads at 1, 1.95 and 2.9 times Time (the spacing of a 1970s three-head echo), averaged. Spread pans the first two apart (the third stays in the middle).
- **Wow and flutter:** wow is a 0.7 Hz sine plus a drift that wanders to a new random target each cycle (60/40), up to 1.8 ms of delay swing at 100 %; flutter is two small sines at 6.3 and 11.7 Hz (the second at half depth), up to 0.12 ms. fxlab measures 0.33 % peak at the defaults (25 % each) and 1.3 % at 100 %, the range from a good cassette deck to a worn machine (research-delay.md: about 0.02 % for a studio machine, 0.08 % for a good cassette deck). At 0 % the speed is steady to 0.0001 %.
- **The read:** each head reads through `kit::FlatRead`, flat while the tape runs steady (time settled, Wow and Flutter at 0) and the cubic while anything moves. The heads read the record stage's 1.3-frame lag sooner, so a repeat lands on its time.
- **Level:** the echoes play 2.5 dB up (`TAPE_TRIM`, outside the loop), so the first echo, darker for its pass, is about as loud as the dry at the defaults.
- **Time:** 20 ms to 1.3 s (the longest a tape loop of that kind gives), or synced from 1/16 to 1 bar with dotted and triplet values; changes glide and slide in pitch, as the tape speed would.
- **Tail:** the passes round the farthest head until the loop has lost 60 dB at its greatest gain (Feedback times the most a pass of the tone lets through, read from the filters on a 1/8-octave grid from 20 Hz to 2 kHz), at most 30 s; a loop whose gain is 1 or more rings on until Feedback comes down.
- **Turned down:** a hysteresis model of the tape (needs a solver per sample); a separate playback-head EQ (one pass of colour per trip round the tape is what is heard); BBD-style companding (a different machine; a possible later mode).

### Multi-tap

- Four taps, each at 1 to 32 sixteenths of the tempo, with level and pan. Only tap 4 feeds back, through Tone (500 Hz to 20 kHz) and Low cut (20 Hz to 1 kHz), up to 95 %, so the pattern repeats as a whole. Each tap reads through `kit::FlatRead`. The tail is the farthest tap plus the passes round tap 4 until Feedback has lost 60 dB, at most 30 s.
- It has no saturation: a pattern of clean taps is its job; Digital and Tape do dirt.

### What every engine shares

- **Mix curve** (`kit::Blend`): Equal (default), Dry first or Linear, gliding the two gains (20 ms on Digital and Multi-tap, 30 ms on Tape). Mix 0 is the input bit for bit and 100 % the effect alone on every curve.
- **Smoothing:** every continuous control glides (20 to 60 ms one-poles); a still control costs a compare (`kit::Glide`).
- **No allocation in `process`** (the lines are sized in `prepare` for 4 s at the rate), **exact silence** once the tail is done (denormals flushed in the loop), and the same output at any block size (the shared gates in `mpfree-fx/tests`).

## 5. Controls

| Engine | Control | Range (default) | Law and behaviour |
|---|---|---|---|
| Digital | Sync | Free, 1/32 to 1 bar with dotted and triplet (1/8D) | Choice; synced time follows the tempo |
| | Time | 1 ms to 4 s (375 ms) | Log; used when Sync is Free; glides 60 ms (pitch slides) |
| | Feedback | 0 to 110 % (35 %) | The loop's gain below 100 %; over it the saturator bounds the howl |
| | Ping-pong | Off, On (Off) | Toggle |
| | Tone | 500 Hz to 20 kHz (7 kHz) | Log; the loop's low-pass, so each repeat darker |
| | Saturation | 0 to 100 % (20 %) | Up to 18 dB of drive into the soft curve, unity on quiet repeats |
| | Mix, Mix curve | 0 to 100 % (30 %); Equal | See section 4 |
| | Freeze | Off, On | Holds the buffer as a loop |
| Tape | Time, Sync | 20 ms to 1.3 s (330 ms); Free, 1/16 to 1 bar | As Digital, glides and slides |
| | Feedback | 0 to 150 % (45 %) | The loop's gain (the saturation never adds any) |
| | Heads | One, Two, Three (One) | Heads at 1, 1.95, 2.9 × Time |
| | Wow, Flutter | 0 to 100 % (25 %, 25 %) | 0.33 % peak at the defaults, 1.3 % at full |
| | Saturation | 0 to 100 % (35 %) | Drive 1 + 4s into tanh, blended by s |
| | Tone, Low cut | 800 Hz to 16 kHz (5 kHz); 20 to 800 Hz (120 Hz) | Log; one pass per trip round the tape |
| | Spread | 0 to 100 % (50 %) | Pans the first two heads apart |
| | Mix, Mix curve | 0 to 100 % (30 %); Equal | |
| Multi-tap | Tap n time, level, pan | 1 to 32 sixteenths (3, 6, 10, 16); 0 to 100 % (80, 65, 50, 40); -100 to 100 % (-60, 60, -30, 30) | Stepped time, following the tempo |
| | Feedback | 0 to 95 % (30 %) | Through tap 4 |
| | Tone, Low cut | 500 Hz to 20 kHz (8 kHz); 20 Hz to 1 kHz (80 Hz) | Log |
| | Mix, Mix curve | 0 to 100 % (30 %); Equal | |

Every new control is appended (ids only grow), so a project saved before it loads at its default; Mix curve's default, Equal, was the one exception to "the default keeps the old sound", by Jay's decision (fx-quality.md, PR B).

## 6. Defaults and presets

The defaults predate the quality work; it kept them because each sits where the research says the job lives, and moved only what measured wrong.

- **Digital at a dotted 1/8, 35 % feedback, Tone 7 kHz, Saturation 20 %, Mix 30 %:** the dotted eighth is the most common rhythmic delay in pop and dance music; 35 % gives three or four audible repeats (the fourth about 36 dB down) before the next bar; 7 kHz puts the repeats just behind the dry; a fifth of the saturation range only touches loud repeats. Mix 30 % on the equal-power curve plays the dry at -1.0 dB and the echo at -6.9 dB, an insert level from the research's 10 to 30 % range.
- **Tape at 330 ms, one head, 45 %, 25 % wow and flutter, 35 % saturation, Tone 5 kHz:** a classic single-head echo that is audibly tape (darker, slightly unsteady) at its defaults, since that is why someone picks the Tape mode; 330 ms is a free time near a 1/4 note at 180 BPM or an 1/8 at 90, a usable echo without the grid.
- **Multi-tap at 3, 6, 10 and 16 sixteenths, panned left, right, left, right with falling levels:** a pattern that is obviously more than one echo and walks across the stereo field.

The presets are one per job in section 1, named for what they do:

| Engine | Preset | Settings | Job |
|---|---|---|---|
| Digital | Slapback | Free 110 ms, no feedback, Tone 6 kHz, Saturation 30 % | Slapback |
| | Dotted ping-pong | 1/8D, 45 %, ping-pong | Rhythmic echo, wide |
| | Triplet bounce | 1/8T, 35 %, ping-pong, Tone 8 kHz | Rhythmic echo, swung |
| | Quarter repeats | 1/4, 30 %, Tone 9 kHz, Saturation 10 % | Rhythmic echo, plain |
| | Dub echo | 1/4D, 70 %, Tone 2.5 kHz, Saturation 50 % | Dub and throws |
| | Tape runaway | 1/8, 95 %, Tone 3.5 kHz, Saturation 70 % | Throws on the edge of howling |
| Tape | Slapback | 110 ms, 10 %, Wow 10 % | Slapback with a little tape |
| | Dub echo | 1/8D, 80 %, Tone 2.5 kHz, Saturation 55 % | Dub that dies away |
| | Space heads | 180 ms, three heads, 60 % | The three-head echo's rhythm |
| | Worn tape | Wow 80 %, Flutter 70 %, Tone 2 kHz | Old, unsteady |
| | Runaway | 1/4, 130 %, Saturation 70 % | A loop that grows into the ceiling |
| Multi-tap | Dotted bounce | 3, 6, 9, 12 sixteenths | Dotted rhythm |
| | Triplet scatter | 2, 5, 7, 11 sixteenths, 45 % | Off-grid scatter |
| | Wide ping | 4, 8, 12 sixteenths, hard left and right | Width |
| | Build up | Levels 30 to 90 %, 60 % | Echoes that grow |

A preset sets the sound, never Mix or Mix curve (`page_keeps`), so a preset on a return stays at 100 % wet. The Tape's Runaway and the Digital's Tape runaway are the presets that take the loop to its limit; fxlab's D4 rows check them every run.

## 7. Measuring quality

`fxlab quality --class delay` measures 80 numbers on every engine at its defaults and every preset, at 44.1 kHz (the MPC's rate), with the loop "open" where a row needs it (Tone at its brightest, no saturation, wow or flutter, one head, Feedback 80 %, Mix 100 %; the Multi-tap with only tap 4). The ratchet (`rust/mpfree-fxlab/quality/ratchet.tsv`, checked on CI) fails a pull request that makes any of them worse. The targets, from fx-quality.md:

| Id | What | Target | Today |
|---|---|---|---|
| D1 | First repeat's time (its group delay, 1 to 4 kHz) | Within 0.5 frame | Digital 0.013, Tape 0.41 (the record pass's own group delay) |
| D2 | Extra loss per repeat half a frame off a whole frame | ≤ 0.1 dB at 10 kHz, ≤ 0.3 at 15 kHz | Under 0.01 dB; Tape 0.17 dB at 15 kHz |
| D3 | The loop's filters equal their drawing | Within 0.5 dB | Unit test (`tests/delay.rs`) |
| D4 | Runaway: peak and DC at the highest Feedback | ≤ +6 dBFS, DC ≤ -60 dBFS | Met (the Tape at 150 % peaks at -2.8 dBFS) |
| D5 | Worst alias of a 5 kHz full-scale sine | ≤ -70 dB, every preset | Under -87 dB |
| D6 | Loudness against the dry at every Mix | -1.0 to +1.5 LU | Met but the Multi-tap's bump, 1.69 LU |
| D7 | Zipper: the sharpest step while a control sweeps | ≤ 3 dB | Unmet on Time sweeps of the Digital's Slapback and four Tape settings (a time change slides in pitch by design) |
| D8 | The Tape's first echo carries the tape's colour | ≥ 40 % of the second's darkening | 0.50 |
| D9 | Wow and flutter at 0 | ≤ 0.01 % | 0.0001 % (defaults 0.33 %, full 1.3 %, reported) |
| D10 | Ping-pong alternates | Each repeat ≥ 20 dB on its side | 60 dB |
| D11 | Mono sum of the wet | Within 3.5 LU of stereo | Met |
| D12 | Level when the slot stops processing | ≤ -60 dB under the input | Met; endless loops (Feedback ≥ 100 %, Freeze) ring on by design |
| D13 | Exact silence after the tail | Zero, no denormals | The shared gates (`idle_db`) |
| D14 | Cost | The instruction gate | Section 8 |

Unit tests: `tests/delay.rs` (the first repeat on its time, D1; the band kept at any fraction of a frame, D2; each pass round the tape is the tone the page draws, D3; the tape's first echo has been round it once, D8; the Tape's Feedback is its loop gain; Digital's tail follows the tempo), `tests/echo_tails.rs` (every tail covers its echoes down to -60 dB; a loop that gains rings on until Feedback comes down), `tests/fx.rs` (synced time on the grid and following the tempo, ping-pong alternating, full Feedback bounded by the saturator, ringing on after bypass and then resting), `tests/mix_curve.rs` (the three curves exact, Mix 0 the input bit for bit, a curve change glides), and the registry-wide gates for block size, extremes, NaN and allocation.

Listening: each sound-changing PR has a blind, loudness-matched A/B page (before against after) for Jay; numbers decide what is fixed, listening decides whether the fix is better.

## 8. Cost

Instructions per 128-frame block at 44.1 kHz, counted by callgrind on CI (`tools/ci/instr_gate.py`, which fails a PR whose case rises more than 5 % unless the rise is explained and blessed; `rust/mpfree-bench/instructions.txt`):

| Case | Before PR C | Now |
|---|---|---|
| Delay (Digital, defaults) | 72.5k | 70.9k |
| Tape delay | 70.1k | 87.9k |
| Multi-tap delay | 89.2k | 95.2k |

The flat read costs less than the cubic, which paid for Digital's oversampled saturator. The Tape's rise (1.25 times, inside the plan's 1.3) is its oversampled record stage. Every control resting costs a compare; the cubic and the all-pass both run only during a 3 ms swap.

## 9. Pitfalls and how the design avoids them

| Pitfall | Avoided by |
|---|---|
| Repeats darker or brighter as the tempo changes (interpolation loss compounding) | The flat all-pass read while the time holds still (D2) |
| Mud from stacked low end | The fixed 60 Hz high-pass in Digital's loop; Low cut on Tape and Multi-tap |
| Runaway, DC or full-scale clipping | Saturation with a ceiling in the loop; DC measured (D4) |
| Aliases that circle the loop | Saturation at twice the rate (D5) |
| A tape echo whose first repeat is clean and the second suddenly dark | The colour at the record head (D8) |
| Clicks when the time moves | The time glides; the read swaps to the cubic while it moves |
| The track getting quieter as Mix turns | The equal-power Mix curve (D6) |
| Echoes cut off when the slot is bypassed | Tail estimates from the loop's real gain (D12) |
| Mono collapse of ping-pong | Ping-pong alternates whole repeats rather than phase tricks (D11) |
| A preset that resets a return to 30 % | Presets never set Mix or Mix curve |

## 10. History and next

- **Before 16 Nov:** the three engines as built by the FX thread (Digital with sync, ping-pong, filtered and saturated feedback and freeze; Tape with three heads, wow and flutter; Multi-tap on the sixteenth grid), all on a cubic read with a linear Mix.
- **PR B (16 Nov, #423):** the Mix curve on every delay and reverb, Equal by default; the Tape's and Multi-tap's tails from their Feedback instead of a fixed 8 s.
- **PR C (16 Nov, #430):** the flat read (`kit::FlatRead`), the loop saturation at twice the rate (`kit::Os2`), the Tape's colour at the record head and a Feedback that is its loop gain (range to 150 %). 55 of 81 targets met before any fix, 74 of 80 after.

**Still to do** (plan.md section 5.2, each a new control appended so saved projects keep their sound):

1. **Time mode:** Repitch (today's behaviour), Fade (two read heads crossfaded over about 40 ms, no pitch change) and Jump. Fixes the D7 Time-sweep rows.
2. **Ducking** on all three: the repeats dip while the dry plays, then swell back.
3. **Low cut on Digital** (20 Hz to 1 kHz, default 60 Hz, today's fixed value).
4. **Modulation on Digital** (Rate and Depth, a gentle chorus on the repeats).
5. **Left-right offset on Digital** (doubling and width).
6. **A level trim on the Multi-tap's taps** (its Mix bump, D6, is 0.19 LU over the target).
7. Later, if wanted: groove (swung echoes), diffusion, a bucket-brigade mode.

## 11. Sources

- Laakso, Välimäki, Karjalainen and Laine, "Splitting the unit delay: tools for fractional delay filter design", IEEE Signal Processing Magazine, 1996.
- Wallach, Newman and Rosenzweig, "The precedence effect in sound localization", American Journal of Psychology, 1949; Litovsky, Colburn, Yost and Guzman, "The precedence effect", JASA, 1999.
- Wow and flutter measurement: IEC 386, DIN 45507, AES6-2008 (a 3.15 kHz tone, a shared weighting).
- Soundtoys EchoBoy (Sound On Sound review); ValhallaDSP, "ValhallaDelay: the MODE control" (2019); Ableton Delay's Repitch, Fade and Jump modes; the Elektron Digitakt manual's delay parameters.
- Arnardottir, Abel and Smith, "A digital model of the Echoplex tape delay", AES 125th Convention, 2008; Chowdhury, "Real-time physical modelling for analog tape machines", DAFx 2019.
- Antiderivative antialiasing: Parker, Zavalishin and Le Bivic, "Reducing the aliasing of nonlinear waveshaping using continuous-time convolution", DAFx 2016; Bilbao, Esqueda, Parker and Välimäki, "Antiderivative antialiasing for memoryless nonlinearities", IEEE SPL, 2017.
- Mix laws: forum threads on Fractal's, JUCE's and Vital's dry and wet curves (research-delay.md lists the links).
