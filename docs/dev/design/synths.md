# Synths: the node interface, Poly, FM, Drums, Pluck, Wavetable and pad pressure

**Status:** 2026-11-11, host-only and tested. The code is [`rust/mpfree-node`](../../../rust/mpfree-node) (the interface every instrument and effect uses) and [`rust/mpfree-synth`](../../../rust/mpfree-synth) (native synths: Poly, FM, Drums, Pluck and Wavetable; Drums and Pluck merged in PR #97, Wavetable in PR #99). Poly plays on the pads (see "In the pad player"); FM, Drums, Pluck and Wavetable are not on the pads yet, and none has run its own cost bench on the MPC. The full design, the roadmap and the mockups are in the design doc "MPFree synths and plugins: design" and in mpfree-claude (`synths/mockups/`, `skill-tree/synth-skill-tree.html`).

## The node interface (`mpfree-node`)

One trait, `Node`, for anything that runs on the audio thread on a stereo bus: a synth, the sampler (later), a hosted plugin (later), an effect (the FX work builds `mpfree-fx` on it).

- `process(ctx, events, left, right)` once per block. `events` are `Timed { offset, ev }`, sorted, with offsets inside the block: `NoteOn`, `NoteOff`, `Pressure` (poly aftertouch), `Bend`, `AllOff`, `Param { id, value }` (automation, parameter locks, macro moves) and `Expression { note, kind, value }` (one note's own slide, tone or release velocity; see [Tunings and per-note expression](#tunings-and-per-note-expression)).
- `set_tuning(&Tuning)` hands an instrument its tuning between blocks. The node copies it (plain data, no allocation), so it may run on the audio thread. The default does nothing, so effects and plugins ignore it.
- `Ctx` carries the rate, the absolute frame of the block's first frame, the tempo, the transport position in beats, whether it plays, and an optional sidechain.
- `params()` describes every parameter with `ParamInfo`: id, name, group, range, default, unit, curve (linear, log, stepped, named choices, toggle) and a read-only flag for meters. A GUI page, Q-Link mapping, automation and p-locks can be built for any node from this list alone. `check_params` checks a list.
- `prepare(rate, max_block)` runs on the control thread and may allocate. `reset()` silences the node and keeps its parameters. `tail_frames()` and `latency()` tell the host how long the node rings and how much delay it adds.

Rules every node keeps:

- Stereo `f32`, in place. Instruments **add** into the buffers; effects transform them.
- 1.0 is 0 dBFS. No node clamps or saturates unless that is its job, so deliberate distortion stays possible. Instruments aim for about -12 dBFS peak for one full-velocity note at their defaults and fade every voice's end. The master node catches accidents with a limiter that counts overs and never errors or mutes.
- No allocation, free, lock, block or panic in `process`.
- The same output, sample for sample, for any block size from 1 to `MAX_BLOCK` (8192) frames. Control-rate work is aligned to the absolute frame, never to the block's start.
- Parameter values are plain units, clamped, never refused; each node smooths its own continuous parameters (`Smooth`).
- A node must not output non-finite samples. If one does, the host resets it and counts it.
- `mpfree_node::pressure` is the shared pad-pressure shaper every engine runs pressure through (see [Pad pressure](#pad-pressure-poly-aftertouch)).

## Poly (`mpfree-synth`)

A subtractive synth:

- **Oscillators:** two per voice (saw and pulse band-limited with PolyBLEP, triangle, sine), octave, semitone and fine tune, pulse width; a square sub one octave down; white noise.
- **Unison:** 1 to 4 stacks per voice, detuned symmetrically and spread across the stereo field. Each voice has a left and a right filter, so the spread stays stereo.
- **Drive and filter:** a `tanh` drive (0 dB is fully clean) into a zero-delay-feedback ladder (LP 24) or a trapezoidal state-variable filter (LP 12, BP 12, HP 12). Cutoff follows the filter envelope, key tracking, velocity, the LFO and pad pressure (see [Pad pressure](#pad-pressure-poly-aftertouch)).
- **Envelopes:** filter and amp ADSR with exponential segments; a retrigger starts from the current level, so it never clicks.
- **LFO:** sine, triangle, saw, square or sample-and-hold, free (0.05 to 30 Hz) or synced to the transport (4 bars to 1/16 triplets, phase-locked to `Ctx::beats` while playing), to pitch, cutoff, pulse width and level.
- **Voices:** poly (1 to 16), mono or legato, glide, bend range. A stolen voice fades out over 3 ms in one of 4 spare slots. Stealing takes the quietest releasing voice, then the oldest held one. Mono and legato keep a note stack.
- **Presets:** Init, Grit Bass, Acid, Supersaw, Dust Pad, Pluck, Brass, Sub Bass and Lead.

Timing: oscillators, drive, filters, the amp envelope and the steal fade run every frame. Smoothing, the LFO, the filter envelope, pressure, glide, pitch and the filter coefficients run once every 16 frames, on frames divisible by 16. Each voice's gain is ramped across a step, so a pressure swell has no zipper.

Cost (after round 7's device bench): each voice renders a control step in passes (gains, each unison stack, sub and noise, drive, filter, out), so the choice of wave, filter type and drive is made once per step, not once a frame. A centred voice (one stack, or no spread) works out one side and uses it for both, since its left and right are the same to the bit. Pitch, pan and filter coefficients are only worked out again when their inputs move. The output is the same to the bit as before; `cargo run --release -p mpfree-synth --example poly_cost` prints a hash per preset to check that after any change. Under qemu, the ARMv7 build runs 12 to 37 % fewer instructions per preset (Init 30 %, Supersaw 20 %, Dust Pad 12 %).

## FM (`mpfree-synth`, `Fm`)

A six-operator FM synth in the DX7's mould. It is DX7-style, not a bit-exact clone: the numbers come from published descriptions of the DX7 (the operator level table, the envelope rate formula, the keyboard scaling curves), and where the original is not public (the LFO speed curve, the pitch envelope's tables, detune) the values are close approximations.

- **Operators:** six sines, each with a ratio (coarse 0 to 31, fine) or a fixed frequency, detune, an output level (0 to 99), a four-rate, four-level envelope, velocity sensitivity, keyboard level scaling (break point, left and right depth and curve), rate scaling and amplitude modulation from the LFO. Each operator can be switched off.
- **Algorithms:** the DX7's 32 ways of wiring them, with feedback 0 to 7 on one operator or, for algorithms 4 and 6, around a loop of two or three. Feedback averages the last two samples, as the DX7 does, so it stays stable at 7.
- **Envelopes:** in decibels, falling at a constant rate, with the attack curving over as it nears its level. Rates and levels from 0 to 99 mean what they mean on a DX7.
- **Global:** a pitch envelope, an LFO (triangle, saw down, saw up, square, sine, sample-and-hold; speed, delay, pitch and amplitude depth, key sync), transpose, oscillator key sync, poly (1 to 16) or mono with a note stack, bend range, pad pressure (see below) and an output level.
- **Level:** one carrier at full level is about -12 dBFS. A voice with more carriers is scaled by 1/sqrt(carriers), so changing the algorithm doesn't jump the volume. The output level glides while notes sound and jumps straight to a new value when nothing sounds, so loading a preset never starts with a swell.
- **Voices:** a stolen voice fades out over 3 ms in one of 4 spare slots, like Poly.
- **Presets:** Init (one sine), Glass Keys, Bell Tower, Rubber Bass, Drawbars, Brass Section, Log Drum, Silk Pad and Steel Pluck. Each plays a chord between -11 and -6 dBFS peak.
- **DX7 banks** (`dx7`): reads a player's own .syx files, a 32-voice bank (4104 bytes) or a single voice (163 bytes), checks the checksum and maps every byte onto FM parameters; it can also write a bank back. Names are 10 characters, as on a DX7, and the bank has no output level, so imported voices play at 0 dB. Only voice data is read; nothing is sent to any device. MPFree ships no Yamaha voices.

Timing: operators run every frame. Envelopes, pitch and the LFO run once every 16 frames, on frames divisible by 16, with operator gains ramped across each step.

**On the Synth tab** FM has its own pages: Algo (algorithm, feedback, transpose, level, voices), Mod (LFO and pitch envelope), Pressure (see below), Op 1 to Op 6 (each operator's frequency, level, envelope and velocity; the Q-Links turn output, coarse, fine and velocity) and Kbd 1-2, 3-4 and 5-6 (keyboard scaling, two operators a page). Every FM parameter is on exactly one page.

Every FM page has a visual strip, drawn from the values:

- **The algorithm** as six boxes: carriers (green) along the bottom over the output line, each modulator (blue) a row above what it modulates, joined by down-across-down lines like the DX7's panel, and the feedback loop on the right when feedback is above 0. A switched-off or silent operator is faded. The layout is worked out from the algorithm's wiring, so all 32 draw the same way (`fm_visual::algorithm_layout`).
- **Algo page:** the algorithm and the six output levels as bars, carriers green.
- **Op pages:** the algorithm with that operator ringed, and its envelope as the engine runs it (L4 up through L1, L2 and L3, held, then the release), with its role in the caption ("modulates 1 · output 58").
- **Kbd pages:** the algorithm with the two operators ringed in their curves' colours, and their keyboard level scaling across A-1 to C7 in dB (the card's scale is the smallest of ±12, 24, 48 or 96 dB that holds both).
- **Mod page:** the pitch envelope (on a ±2, 12, 24 or 48 semitone scale, so a small scoop still shows) and the LFO's wave after its delay.

A tap on a box opens that operator's page; a tap elsewhere on the algorithm card opens Algo. `cargo run --locked -p mpfree-synth-ui --bin mpfree-synth-preview` draws the samples `synth-fm-algo`, `synth-fm-mod`, `synth-fm-op`, `synth-fm-kbd` and `synth-fm-pressure`. FM isn't in the pad player yet; Poly is the pads' synth for round 6.

## Drums (`mpfree-synth`, `Drums`)

A drum synth with a model per pad. Sixteen pads, each one of eleven models: **Kick, Snare, Clap, Closed hat, Open hat, Tom, Rim, Cowbell, Cymbal, Zap and FM perc**. Every pad has the same fourteen knobs, ids `pad * 16 + field` (`drum_params::pad_id`), grouped "Pad 1" to "Pad 16":

- **Model**, **Tune** (±24 semitones from the model's own pitch: kick 50 Hz, tom 120 Hz, snare 185 Hz ...), **Decay** (10 ms to 5 s, the time to fall 60 dB).
- **Tone** (the kick's shape, the snare's and hats' filter, the FM perc's index), **Snap** (a short noise click; for the clap, how sharp its bursts are), **Noise** (the noise layer: the snare's wires, the hats' noise against their six square oscillators).
- **Sweep** and **Sweep time** (a pitch drop from up to 48 semitones above), **Drive** (soft saturation), **Level**, **Pan**.
- **Vel > tone**, **Vel > level** and **Choke** (Off or group 1 to 4).

How it plays:

- **Notes:** pad 1 is the kit's *Pad 1 note* (36 by default, the MPC's usual kick note) and each pad after it a semitone higher; other notes are ignored. Note-offs are ignored too: every hit is a one-shot that rings for its decay.
- **Hits:** a new hit on a pad fades its last hit out over 3 ms. A hit in a choke group fades out the group's other pads, so the closed hat (pad 4) closes the open hat (pad 5) in the Init kit. AllOff fades every hit over 30 ms.
- **Knobs** apply to the next hit, as on drum machines: a hit takes its pad's settings when it starts. The kit Level moves at once, smoothed.
- **Level:** at full velocity the Init kit peaks from about -15 dBFS (hats) to -9 dBFS (the kick). Pressure and bend are ignored.
- **Kits:** Init Kit (kick, snare, clap, closed and open hat, three toms, rim, cowbell, cymbal, zap, FM perc, a long kick, a tight snare, a shaker), Boom Kit (long 808-style booms), Tight Kit (short and bright), Grit Kit (everything driven) and Electro Kit (zaps and FM blips for the toms).

Every part of a voice runs every frame (no control rate), so the output is the same for any block size. 32 hit slots; nothing allocates after `prepare`.

**On the Synth tab** Drums has a page per pad (the tab strip shows 1 to 16) and a Kit page. The Q-Links turn the pad's Tune, Decay, Tone and Snap. `pages::drum_page(pad)` gives the page of a pad, so the app can follow the last pad hit. The preview draws `synth-drums-kick`, `synth-drums-boom`, `synth-drums-hat` and `synth-drums-kit`. `Drums::pad_name(pad)` gives the model name for pad labels.

Its visual strip has four cards. **Kit**: a box per pad, laid out as the pads are (pad 1 bottom left), tagged with its sound (BD, SD, CP, CH, OH, TM, RS, CB, CY, ZP, FM) and coloured by what it is made of (green a tuned body, amber noise over a body, blue metal); the pad shown is ringed, a silent pad faded, and a tap on a box opens that pad's page. **Hit**: the pad's body, noise (a clap's four bursts over its tail) and click over time, each at its level in the mix, until the longest is 60 dB down. **Sweep**: the body's pitch sweep over 250 ms, against the knob's 48 semitones ("no body to sweep" for the hats, clap, cowbell and cymbal). **Levels**: every pad's level, the pad shown in white. On the Kit page Hit and Sweep show pad 1. `drum::hit_shape` works the shapes out with the same code that builds the hits the engine plays (`build`), and a test renders a kick and finds the body's fall and the sweep's pitch as drawn.

## Pluck (`mpfree-synth`, `Pluck`)

Plucked strings and struck bars: two physical models behind one set of pages, picked with **Model**.

- **String** is a Karplus-Strong string (the Jaffe-Smith extension): a delay line one period long with a one-pole damping filter and a first-order allpass in the loop. The tuning takes the filter's phase delay into account, so notes from C1 to C7 play within 2 cents (within 6 at C1 for the darkest presets). A pluck fills the line with a burst of noise shaped by **Pick** (brightness of the burst) and **Position** (a comb: plucking near the bridge is thin, near the middle hollow). **Brightness** sets the loop filter between 12 and 112 harmonics of the note.
- **Mallet** is a modal bank: up to eight two-pole resonators per voice at a **Material**'s mode ratios (Marimba, Vibes, Bell, Glass, Kalimba, Steel drum), struck by a half-sine mallet pulse. **Hardness** is the pulse length (a harder mallet reaches higher modes); **Strike** is where the bar is hit, **Stretch** spreads the ratios, **High damp** shortens the upper modes.
- **Decay** is the ring at middle C (higher notes ring shorter, lower ones longer); **Release** is how fast a note damps once its key is let go (1.5 s by default, so pad taps ring). **Vel > tone** makes soft notes darker. Transpose, Fine, Voices (1 to 16), Bend range, Level, Pan and Spread (alternate voices left and right).
- **Notes:** playing a note that is still ringing re-plucks it in its own voice, adding to what sounds, as a real string does. Past the voice limit the oldest note (released ones first) fades out over 3 ms. Pressure is ignored.
- **Level:** every preset peaks between -16 and -9 dBFS at full velocity across C2 to C6.
- **Presets:** Init, Nylon, Steel String, Harp, Koto, Muted Bass (strings), and Marimba, Vibes, Bell, Glass, Kalimba and Steel Drum (mallets).

Timing: the strings and resonators run every frame; pitch (transpose, fine, bend) runs at a control step of 16 frames aligned to the absolute frame. The delay lines (one per voice, 4096 frames at 48 kHz) are allocated in `prepare`.

**On the Synth tab** Pluck has three pages: String (with the model, decay and release; Q-Links Decay, Brightness, Pick, Position), Mallet (Q-Links Hardness, Strike, Stretch, High damp) and Voice. The preview draws `synth-pluck-string` and `synth-pluck-mallet`.

Its visual strip has three cards for the model it plays, all for middle C. A string: **Pick** (the string pulled up where it is picked), **Harmonics** (the first 16, in dB, at their average level over many plucks, since each pluck's noise burst gives them random levels around it) and **Decay** (harmonics 1, 2, 4 and 8 falling in dB, higher ones sooner, more so when darker). A mallet: **Mallet** (the pulse, as long as the mallet stays on the bar), **Modes** (the material's modes on a log scale of the note, as struck: placed by Strike, weighed by the pulse's spectrum) and **Decay** (the first four modes). `pluck::partials` works them out with the engine's own formulas (shared helpers, the output unchanged to the bit), and tests render notes and find each harmonic's and mode's decay, and the harmonics' average levels, as drawn. One finding: Position's comb works within the burst's one period (the first frames are not subtracted), so its notches are partial; plucked at the middle, the even harmonics come out about 7 dB under the odd ones on average rather than gone.

## Wavetable (`mpfree-synth`, `Wavetable`)

Two table oscillators that scan through stacks of single-cycle frames, with warps (Bend, Sync, Mirror, FM), unison, a sub and noise, Poly's filter, a mod envelope and LFO that move the positions, and pad pressure. Ten factory tables are built from formulas; a player's WAV tables and a pad's sample can become user tables. Tables are band-limited at 17 half-octave levels, so high notes don't alias. The ten factory tables are built once, by the first `Wavetable::new` (the pad player makes it before the sound starts): about 0.9 billion ARMv7 instructions under qemu, most of them FFTs, so it adds to start-up time, never to a block. The full description, the memory and cost numbers and the demo are in [Wavetable synth](wavetable.md).

## Pad pressure (poly aftertouch)

The MPC One's pads send pressure for each pad while it's held. Players who have per-note pressure rarely give it up, and most complaints about it are about feel: it fires by accident, jumps when a note starts or ends, moves in coarse steps, or can't be tuned. So every engine shapes it the same way before it reaches a sound. The research and the reasoning are in the design doc [Poly aftertouch for MPFree synths](https://claude.ai/artifact/7SWqijpsJoBR2LjEHkNU3m).

**The shaper** (`mpfree-node`, `pressure`) runs once per voice every 16 frames. The sampler can use it without depending on the synths.

1. **Hold-off** (0 to 100 ms, 30 by default): for this long after a note-on the value holds still, so the strike itself doesn't modulate. A fresh voice holds at 0 and then glides in; a voice retriggered while it sounds keeps its value, so it doesn't dip.
2. **Threshold** (0 to 50 %, 8 by default): below it the output is exactly 0, so a resting finger does nothing. The scale starts again from 0 at the threshold, with no step.
3. **Full at** (30 to 100 %, 85 by default): the raw pressure that gives the full effect.
4. **Curve**: Soft, Linear, Firm (the default) or Hard, powers 0.6, 1, 1.6 and 2.5. Steps 2 to 4 are a 128-entry table worked out when a setting changes.
5. **Rise and fall** (0 to 200 ms each, 8 and 40 by default): one-pole smoothing, quicker up than down.
6. **Release**: Hold (the default) keeps the value the voice has at its note-off; the 40 ms fall means a lifting finger has barely lowered it. Follow keeps following the pad. Fade falls to 0.
7. **Mode**: Per note (the default), Highest (every voice follows the highest pressure among the held notes, so a chord swells together without jumps) or Off. Off makes pressure do nothing: a level swell then plays at the preset's full level.

**Slots.** Each engine has four slots, each a target and a depth of -100 to 100 %. A depth reads in its target's unit on the Synth tab (`mpfree-synth`, `pressure::full_scale`):

| Target | 100 % is | Poly | FM |
|---|---|---|---|
| Cutoff | 4 octaves | yes | |
| Resonance | 100 % | yes | |
| Level | a 24 dB swell | yes | yes |
| Vibrato | 1 semitone (Poly: its own sine at Vib rate, 5.5 Hz; FM: its LFO, without the LFO delay) | yes | yes |
| LFO rate | 2 octaves faster; the LFO is shared, so it follows the highest pressure | yes | yes |
| Pitch | 12 semitones | yes | yes |
| Drive | 24 dB more | yes | |
| Width (pulse) | 45 % | yes | |
| Sub, Noise | 100 % of their level | yes | |
| Brightness | 24 output-level steps (18 dB) on every modulator | | yes |
| Tremolo | a 100 % dip, at the LFO's rate | | yes |
| Feedback | 7 steps | | yes |

**Level is a swell, never a boost.** A positive depth lowers the voice by the depth at zero pressure and plays it at the preset's level at full pressure; a negative depth ducks it as you press. Pressure on level never pushes a patch past the level it was designed at, so it causes no unintended clipping. Deliberate grit goes through Drive.

**Presets.**

| Preset | Pressure |
|---|---|
| Brass (Poly), Brass Section (FM) | a 9 dB swell; +1.5 oct cutoff (Poly), +12 steps brightness (FM) |
| Dust Pad (Poly), Silk Pad (FM) | a 6 dB swell; 0.3 semitones of vibrato |
| Lead (Poly) | 0.5 semitones of vibrato; +1 oct cutoff |
| Acid (Poly) | +2 oct cutoff; +20 % resonance |
| Init, Supersaw (Poly) | +1 oct cutoff (the old fixed setting) |
| Grit Bass, Sub Bass (Poly) | +6 dB drive |
| Rubber Bass (FM) | +8 steps brightness (FM has no drive) |
| Drawbars (FM) | LFO rate twice as fast, like a rotary speaker speeding up |
| Init, Glass Keys, Bell Tower (FM) | vibrato, 0.18, 0.2 and 0.2 semitones |
| Pluck (Poly), Steel Pluck, Log Drum (FM) | none: these sounds decay, and pressure would fight the hit |

A swell preset at rest (a pad not pressed, a keyboard without aftertouch) plays quieter by the swell's depth. Mode Off plays it at its full level.

**Ids.** Poly's pressure block is ids 100 to 115 and Vib rate is 75; FM's block is ids 224 to 239. The old fixed amounts (Poly 74, pressure to cutoff; FM 22, pressure to vibrato) are retired, and slot 1 does their job by default.

**On the Synth tab** both engines have a Pressure page: the 8 settings and 4 slots, a drawing of the curve with the threshold and full-at points marked, and a live bar for each sounding voice's pressure. The app passes those values in with `SynthPanel::set_pressures`, from `Poly::pressures` or `Fm::pressures`. Q-Links 1 to 4 turn the threshold, full at, depth 1 and depth 2. With twelve pages, FM's tabs get room by the length of their names.

**What other parts own.** Per-pad calibration is the app's: it scales each pad's raw value before the event reaches a node. Recording pressure into clips, Note Repeat and the sampler pads adopting the shaper belong to the time engine, the arp and the sampler. How the One's pads actually send pressure (rate, a spike on the strike, a 0 before the note-off, evenness) still needs a read-only measurement round before the defaults are final.

## Tunings and per-note expression

**Tunings** (`rust/mpfree-tuning`). A `Tuning` turns a note number, fractional for bends and slides, into Hz. The default is 12-TET with A4 = 440 Hz, worked out with exactly the operations every engine used before (`440 * exp2((note - 69) / 12)`), so every factory sound is unchanged to the bit: all 38 `engine_cost` and 14 `poly_cost` hashes match main on x86 and on ARMv7.

- **Equal tunings:** any number of steps to the octave, each key one step, A4 on key 69 at any pitch from 100 to 1000 Hz (`Tuning::equal(steps, a4)`).
- **Table tunings:** each key from MIDI -64 to 191 has its own pitch. A note between two keys lies between their pitches on a log scale, so a slide or bend moves evenly from one key's pitch to the next.
- **Built in** (`Builtin`): 12-TET, 5-limit just intonation, Pythagorean, quarter-comma meantone and 24-TET quarter tones. The table tunings take a root (their 1/1, which keeps its 12-TET pitch) and an A4, so "just intonation on A at A4 = 432" is one call. `Builtin::tuning` never allocates (the degrees are a fixed array, `Builtin::degrees`), so the audio thread can switch tunings; it gives the same table, to the bit, as the scale through `scala::tuning`.
- **Scala files** (`scala`): `.scl` scales (cents or ratios, any period, so Bohlen-Pierce works) and `.kbm` keyboard maps (middle key, reference key and frequency, a map per repeat with `x` for unmapped keys). With no map, keys follow the degrees from middle C at its 12-TET pitch, as Scala does. Two differences from Scala, so a note never vanishes: every key follows the map whatever its first and last keys say, and an unmapped key sounds between its neighbours.

Poly, FM, Wavetable and Pluck play every note through their tuning; a sounding note moves to a new tuning on the next control step. FM's fixed-frequency operators stay fixed. Drums ignore tunings (a kit is pitched per pad).

**Per-note expression** (`NodeEvent::Expression`, `mpfree_node::Expr`):

- **Slide:** the note's own pitch offset, -48 to 48 semitones, added to the bend. Only voices playing that note move, so one note of a chord can slide (a sus4 resolving, a pitch lock on one note). Engines glide to each new value over 5 ms (`expr::EXPR_MS`), so a stream of values never zips; a sender ramps a long slide itself.
- **Tone:** 0 to 1, MPE's timbre axis. The engines keep it per voice; routing it to destinations, as pressure is, comes next with a Synth tab page.
- **Release:** release velocity, 0 to 1, for the note's coming note-off. The native engines don't use it yet.
- A value sent before a note-on (as MPE controllers do) is kept for that note and starts it in place. A note-off puts the note back to neutral for its next note-on; the releasing voice keeps its values. AllOff and reset clear every note. With nothing sent, nothing changes to the bit.
- **CLAP plugins** get them as CLAP note expressions: a note's slide is its tuning expression (with the bend added when the bend goes that way), its tone is brightness, and its release velocity goes in its note-off. A new note now also starts at a bend held from before it. The plugin sandbox carries `Expression` across (protocol version 2).

**Not yet.** A tuning per track slot in the track host, Tone routing in the native engines, and MPE from MIDI input (no MIDI port is open yet). The sequencer and the arp can send slides now. Choosing a built-in tuning is on the Synth tab's options (see [In the pad player](#in-the-pad-player-rustmpfree-app)).

## Tests

- `mpfree-node`: curves round-trip, clamping, description checks, event splitting, smoothing. The pressure shaper, 7 tests: below the threshold is exactly 0 and it rises with no step; a strike spike in the hold-off gives nothing, then pressure glides in with no jump; a retriggered voice keeps its value through the hold-off; release Hold, Follow and Fade as a finger lifts before the note-off; no step moves further than the rise or fall time allows; Highest follows the highest held note; the parameter block reads back into the settings.
- `mpfree-synth/tests/poly.rs`, 12 tests:
  - a busy phrase (chords, unison, a tempo-synced LFO, drive, pressure, bend, automation, more notes than voices, AllOff, a switch to legato with glide) gives bit-identical output at 9 block sizes from 1 to 8192 frames and a random sequence of sizes
  - A4 plays at 440 Hz (zero crossings over half a second)
  - one full-velocity note at the defaults peaks between -16 and -8 dBFS
  - voices end after their release and after AllOff, then the output is exactly silent
  - the voice limit holds and stealing makes no click (sines, largest sample step below 0.02)
  - instruments add to what is in the buffer
  - legato glides without retriggering and returns to the held note
  - a low cutoff takes out high-frequency energy
  - every parameter at its minimum and maximum, and everything extreme at once, stays finite
  - parameter descriptions and every preset value are valid
  - every preset plays a chord between -30 and -1 dBFS and is silent at the end
  - a tempo-synced square LFO on the level changes on the beat
- `mpfree-synth/tests/fm.rs`, 18 tests:
  - the same busy phrase test as Poly (chords, feedback, the pitch envelope, the LFO, pressure, bend, automation, more notes than voices, AllOff, mono) is bit-identical at every block size
  - A4 at 440 Hz; the Init voice peaks near -12 dBFS; instruments add to the buffer; voices end after release and AllOff
  - every algorithm has the DX7's carriers, and every route feeds downward or through its feedback loop
  - a modulator adds harmonics and feedback adds more; ratio, fixed and detune frequencies are right
  - a decay falls at a steady rate in decibels; velocity sensitivity scales the level; keyboard level and rate scaling follow the note
  - the pitch envelope starts away from the note and settles on it
  - the voice limit holds and stealing makes no click; mono plays legato and returns to the held key
  - extreme settings stay finite; descriptions and presets are valid; every preset plays a chord at a sane level and ends
  - a DX7 bank written from the presets loads back and plays
- `mpfree-synth/tests/pressure.rs`, 11 tests:
  - every preset of both engines keeps a chord between -30 and -1 dBFS unpressed and fully pressed
  - a level slot is a swell, never a boost: with it, full pressure peaks no higher than without it
  - Pluck, Steel Pluck and Log Drum ignore pressure (bit-identical output)
  - Off plays a preset as if it had no pressure slots
  - per-note pressure moves only the pressed note, and opens its filter
  - a strike spike is ignored and pressure glides in; release Hold keeps the value as the finger lifts, Follow and Fade go to 0
  - vibrato at 100 % moves the pitch a semitone each way, and Pitch at 100 % is an octave
  - Highest swells the whole chord; Off moves nothing
  - FM brightness brightens without getting louder (RMS within 1 dB)
  - bit-identical output at block sizes 1 to 40000 in every mode and release setting, for both engines
- `mpfree-synth/tests/expression.rs`, 6 tests on Poly, FM, Wavetable and Pluck: in just intonation on C, C keeps its pitch while E and G sit at pure 386 and 702 cents (within 3 cents, by autocorrelation), and the standard tuning plays as before to the bit; a tuning change reaches a sounding note; a slide moves its own note (+7 gives a fifth, +12 part way an octave) and a slide for another note changes nothing to the bit; a note-off clears the note's slide for its next note-on; neutral values (a slide of 0, a release velocity, not a number) change nothing to the bit; slides land the same at block sizes 1 to 4096.
- `mpfree-tuning`, 16 tests and a no-allocation test (built-in tunings and rooted scales): every built-in tuning on every root at five A4s matches its scale through `scala::tuning` to the bit, and is worked out with no allocation; the standard tuning is the old formula to the bit across notes -200 to 300; equal tunings, A4 and clamping; just, Pythagorean and meantone intervals are pure where they should be; notes between keys lie between their pitches; every note gives a finite frequency; unmapped keys lie between their neighbours; `.scl` reading (cents, ratios, comments, trailing text, Windows line ends) and its errors with line numbers; a 12-TET file matches the standard tuning; a `.kbm` places degrees, the reference pitch and unmapped keys; short maps; map errors; a scale repeating at a twelfth; a rooted scale (12-TET, 7 notes, 1 note, Bohlen-Pierce, on four keys and pitches) is the linear map on that root to the bit.
- `tests/no_alloc.rs`: a counting allocator watches 4000 blocks of random notes, pressure, bend, per-note slides, tones and release velocities, tuning changes, parameter changes over every parameter, AllOff and a reset, and sees no allocation or free. It runs every engine.
- `dsp` unit tests: oscillator bounds, envelope stages, low-pass and high-pass responses.

## Listening and timing

- `cargo run --release -p mpfree-synth --example synth_render [out.wav]` writes `~/mpfree-work/synth-demo.wav`: 9 bars at 92 BPM with synthesised drums, where Grit Bass, Acid (a cutoff sweep by automation), Dust Pad, Supersaw and Lead come in one after another, then a final chord rings out. It prints a timeline, each part's level per bar, checks each part sounds only where it should, and trims the master to -1 dBFS. The parts run through `Node::process` in 128-frame blocks.
- `cargo run --release -p mpfree-synth --example fm_render [out.wav]` writes `~/mpfree-work/fm-demo.wav`: 9 bars at 92 BPM with the same drums. Rubber Bass plays all the way through; Log Drum comes in for bars 3 and 4 (velocity drives its modulator); Glass Keys chords and Brass Section stabs (algorithm 22 and its pitch envelope) take bars 5 and 6; Bell Tower (with a bend down a semitone and back) and Steel Pluck, read back from a DX7 bank written from the presets, take bars 7 and 8; the final chord rings out. It checks the same things as the Poly demo, plus that the end is quiet and no voice is left on.
- `cargo run --release -p mpfree-synth --example pressure_render [out.wav]` writes `~/mpfree-work/pressure-demo.wav`: the same 4-bar phrase twice at 90 BPM with the same notes and the same pad pressure, first with pressure Off on every synth, then on. A Brass chord with only its top note pressed (it swells twice), an Acid line that opens as the phrase builds, an FM Brass Section chord that swells and brightens, and a Dust Pad with vibrato on its top note; a last Brass chord swells and rings out in bar 9. It measures what pressure does in the second pass (the level difference between the passes, and brightness), checks each part clearly moves, and trims the master to -1 dBFS.
- `cargo run --release -p mpfree-synth --example drum_pluck_render [out.wav]` writes `~/mpfree-work/drum-pluck-demo.wav`: 11 bars at 90 BPM. Bars 1-2 are the Init Kit alone (the open hat choked by the next closed hat, a tom fill); then the Grit Kit with the Muted Bass pluck; the Boom Kit with a Koto line; the Electro Kit with a Marimba riff; the Grit Kit with a Bell melody and a Kalimba arpeggio; and a Glass chord and a last boom ring out. Kits change on the bar line while the last hits ring on. It prints a timeline and each part's level per bar, checks each part sounds only where it should and the end is quiet, and trims the master to -1 dBFS.
- `cargo run --release -p mpfree-synth --example tuning_render [out.wav]` writes `~/mpfree-work/tuning-demo.wav` (44 s): C, F, G, C on the FM Drawbars organ in 12-TET, then just intonation on C, then quarter-comma meantone on C (the thirds beat in 12-TET and lock still in the other two); the Poly Dust Pad holding C, E and G while only the E slides up to F and back; and the Pluck Koto playing C major in 12-TET, then maqam Rast on C in 24-TET, with its E and B a quarter tone flat. Tunings go in through `Node::set_tuning` and slides as `Expression` events, in 128-frame blocks.
- `mpfree-bench` has cases for both (module "synth"): Drums Init Kit with a hit every 4 blocks and Boom Kit with a hit every block, and Pluck Steel String (8 voices), Harp (16), Marimba (8) and Bell (16) with chords changing every half second. On the build host (an x86 server core, 44.1 kHz, 128 frames) Drums take about 2 % of a period at p99 and the plucks 0.8 to 1.4 %; the MPC One's numbers need its own run (`--calibration`).
- `cargo run --release -p mpfree-synth --example engine_cost [fm|drums|pluck|wt [PRESET [SECONDS]]]` renders every FM, Drums, Pluck and Wavetable preset (and two Wavetable unison cases: Morph Pad with no spread, and Init at unison 3 with the spread switched on and off) the same way every run, with chords, pressure and a bend (Drums: a 16th pattern over the kit), and prints a hash of each output and the time per block. Like `poly_cost` for Poly, it checks that a change to an engine's code leaves the sound the same to the bit. The same-output pass of 11 Nov kept every hash and cut the instructions an ARMv7 build runs under qemu for 0.6 s (setup included): FM Bell Tower 22 %, Pluck Bell 10 %, Drums Grit Kit 5 %, Wavetable's voice loop 11 % for Morph Pad (unison 3, wide) and 22 % for Digital Bell (centred), its start-up table build left out; Poly's Supersaw another 3 %. What changed: FM turns a phase offset into an integer through an i32 when it fits (the i64 conversion is a library call on ARMv7), reads its sine table without bounds checks, decides once per run which operators sound, and works an operator's increment out again only when its pitch moves; Pluck no longer copies every voice each control step to look at it; Drums floor phases through an i32, skip the exp2 for a body with no sweep and work out the drive's square roots at the hit; Wavetable works out a centred voice (one stack, or no spread) once for both sides, as Poly does, with the centred and stereo loops as separate copies; the noise source converts through a u32; the engines' cache keys compare in line rather than through a `bcmp` call. Drums still spend most of their time in `tanhf` (drive) and `exp2f` (the sweep); those library calls can only go if the sound may change slightly.
- `cargo run --release -p mpfree-synth --example synth_bench` times one 128-frame block with 1 to 16 voices: Poly at unison 1 or 4, and FM's Brass Section and Glass Keys. On the build host (an x86 desktop core) 16 voices take about 4 % of real time for Poly at unison 1, 5 to 6 % at unison 4, and about 3 % for FM; this host's runs vary by up to a third between runs, and pressure made no difference beyond that. The MPC One's Cortex-A17 is slower; it needs its own run before voice limits are set.

## The Synth tab (`rust/mpfree-synth-ui`)

The instrument page for the MPC One's 1280x800 screen, on `mpfree-ui`. It works for any `Node`, so hosted plugins and effects get the same page.

- **Pages.** A native engine names its pages and what its Q-Links turn (Poly: Osc, Filter, Env, LFO, Voice, Pressure; FM: see above). Any other node gets pages from its `ParamInfo` groups: consecutive groups share a page up to 12 tiles, and a group bigger than 16 is split. Every parameter is on exactly one page.
- **Tiles.** Continuous and whole-number parameters are knobs; a bipolar one fills from the top. Choices of up to four show as buttons when they fit, otherwise as the chosen name with arrows. Toggles are switches. Read-only parameters are live meters. A tile's label carries its group when the group isn't the page's ("Env · Attack" on the Filter page).
- **Visuals.** Poly gets four cards drawn from its values: both oscillators, the filter response (with a dashed line where the filter envelope takes it), the amp and filter envelopes, and the LFO. A tap on a card opens its page. FM gets the algorithm diagram and a card for each page (see above). Drums gets the kit, the hit, the sweep and the levels; Pluck the pick or mallet, the harmonics or modes, and their decays (see their sections; `kit_visual`). On the Pressure page (Poly and FM) the strip shows the pressure curve and the live bars instead. Other nodes get the plain layout without cards.
- **Control.**
  - A finger dragged up or down a knob turns it; 320 px is the whole range.
  - A tap selects a tile, and the wheel then turns it.
  - Q-Links 1 to 4 turn the marked tiles. A detent is one step for whole numbers and choices, and 1/100 of the range along the parameter's curve otherwise.
  - SHIFT + wheel steps through the pages.
- **Q-Link sets** (`qsets`). A sound carries named sets of four controls, so the Q-Links can play it from any screen without opening its editor. Each engine has curated sets, in the order a player reaches for them:

  | Engine | Sets |
  |---|---|
  | Poly | Tone (cutoff, resonance, filter env amount, drive), Env, Filter env, Motion (LFO rate, LFO to cutoff, LFO to pitch, unison detune), Mix, Osc |
  | FM | Algo (algorithm, feedback, transpose, level), Levels 1-4, Ops 5-6 (levels and coarse), Ratios 1-4, Mod (LFO) |
  | Wavetable | Tables (both positions and warps), Tone, Env, Mod env, Motion, Mix |
  | Pluck | String, Mallet, Play |
  | Drums | Hit (tune, decay, tone, snap), Body, Mix, on the pad hit last |

  Any other node, such as a plugin, gets one set per Synth tab page (at most 8). Drums' sets name pad 1's parameters; `qsets::on_pad` moves them to the pad hit last, and `qsets::pad_relative` keeps a knob learned on another pad as pad 1's, so it follows the pad too. `qsets::save` writes the sets as `qset = <id> <id> <id> <id> <name>` lines for a preset file (`-` for a free knob), and `qsets::load` reads them back from a whole preset file: an id the node no longer has, or a meter, loads as a free knob. `qsets::to_banks(sets, params, owner)` makes the sets into `mpfree-mod` banks for `Bindings::set_page_sets` (one bank per set, each knob turning its parameter on `owner`, the sound's `Owner::Inst(track)`, over its whole range). After a learn (`Bindings::take_sets_edited`), `qsets::from_banks(info, banks, params, owner)` reads them back to save with the sound: a knob learned to anything that can't travel with the preset (an effect, a macro, another track's sound) is saved free, and a Drums pad's knob as pad 1's. Wiring this into the app's Q-Link column is the app's; nothing on screen changes yet.
- **The app's part.** The app owns the node on the audio thread. It builds a `SynthState` from the node (`SynthState::from_node`) and adds the panel with `SynthPanel::add(ui, "Synth", state)` after its own `header` and `hint` widgets. It passes touches (`SynthPanel::event`), Q-Link turns (`qlink`) and wheel turns (`wheel`, `step_page`) to the panel, and applies each `SynthAction` it gets back: `Set { id, value }` becomes `Node::set_param`, and `Preset(i)` loads the preset, after which the app calls `SynthPanel::load` with the node's new state. The panel updates its own copy of a value at once, so the screen keeps up with a finger.
- **Preview.** `cargo run --locked -p mpfree-synth-ui --bin mpfree-synth-preview` draws the sample screens to PNG files in a new folder `~/mpfree-work/synth-preview-<UTC>`. It opens no device.
- **Tests.**
  - Q-Link sets: every engine's curated sets turn real knobs (no meters, switches or choices, nothing twice in a set), Drums' sets follow the pad, the sets save and load with a preset, old or odd preset lines load safely, the sets go to Q-Link banks and come back unchanged (Drums on any pad too), and a learn on the Q-Links saves with the sound only when it is the sound's own parameter.
  - The anti-aliased drawing, value text, page plans and visual curves (every curve stays in its box for every parameter at its minimum and maximum). All 32 algorithms lay out with no boxes overlapping, carriers on the bottom row and every modulator above what it modulates.
  - `tests/panel.rs` runs a Ui at 1280x800 with a real Poly. It checks that dragging, tapping choices and switches, Q-Links, the wheel and presets reach the node, that every Poly parameter can be reached and set, that the cards follow the page, that the Pressure page draws the curve and live bars and reads depths in their targets' units, that FM draws its algorithm, rings the page's operators and opens an operator's page from a tap on its box, that every FM page has its strip, that a node without a plan gets the plain page, and that every sample screen lays out inside the screen and redraws only the panel after a turn.

## In the pad player (`rust/mpfree-app`)

The pad player (`play` and `app`) carries Poly, FM, Drums, Pluck and Wavetable; one of them has the pads. `src/synth.rs` holds them all on the audio thread; the Synth tab shows the one on the pads. Round 6 brought Poly; FM, Drums, Pluck, Wavetable and the pad layouts came in round 8.

- **Picking it.** The kit list ends with "Synth: Poly" (or "Synth: FM", "Synth: Drums", "Synth: Pluck", "Synth: Wavetable"). Picking it with the wheel or a tap, or opening the Synth tab, hands the pads to the synth. Picking a drum kit hands them back, and the synth's notes end with their release. The sampler keeps its kit underneath, and its sounds finish.
- **The Synth tab's options** (MENU on the Synth tab, `app/synth_pads.rs`): Engine (Poly, FM, Drums, Pluck or Wavetable), Scale (Off, the built-in scales and the owner's), Scale root, Pad layout (for the bank playing), In key rows, Tuning (12-TET, Just (5-limit), Pythagorean, Meantone 1/4 comma, 24-TET quarter tones, then the tuning files), Tuning root (the note the table tunings and files are pure on; "(not used)" for the equal ones) and A4 (380 to 480 Hz, 1 Hz a step), then the Play rows as before. Every engine but Drums plays through the tuning; held notes follow on their next control step. The tuning is worked out on the audio thread without allocating (`TuningPick::tuning`), and the pads' log line adds it when it isn't 12-TET at 440 Hz ("tuned Just (5-limit) on D, A4 432 Hz").
- **Tuning files** (`load_tuning_files` in `main.rs`, `TuningFile` in `synth.rs`): at start the app reads `files/tunings/` in the data store (`Store::list`, read only; names `[a-z0-9._-]`, 1 MiB a file), takes each `NAME.scl` that parses, in name order, at most 64 (`MAX_TUNING_FILES`), with `NAME.kbm` beside it when there is one, and logs `tunings: found dorian (7 notes, with its .kbm), just (12 notes)` and one `left out` line, with the reason, per file it can't use (a scale or map that doesn't parse, a map with no scale). Text that isn't UTF-8 is read lossily, since many published files are Latin-1 and only their descriptions use it. All parsing happens there, before the sound starts. A file with a map keeps the tuning the map gives (`scala::tuning`, which allocates, done once); one without keeps its scale, and the audio thread moves it to the picked root and A4 with `Scale::rooted`, which never allocates and gives the same table, to the bit, as `scala::tuning` with a linear map on that root (a test checks it, and that a `.scl` spelling just intonation plays exactly as the built-in one). The synth holds the list (`Synth::set_tuning_files`, before the sound starts) and the menu's list is built from the synth's, so a pick (`TuningPick::file`, an index) always names the file that plays; a pick the synth doesn't have falls to the built-in tuning and the settings and log line say so. The Tuning row cuts names to 20 characters; with a map, the root and A4 rows read "set by its .kbm". The pick isn't saved yet. On an engine switch the old engine's notes get their note-off and finish their release while the new one plays; each engine keeps its own preset.
- **Notes.** With no scale, pad 1 plays C3 and each pad after it a semitone higher, so the pads play C3 to D#4, in every bank. With a scale the pads follow `mpfree_scale::pads` (see [scales.md](scales.md#the-pads-mpfree_scalepads)): each bank has its own layout, out of the box A Chromatic (the scale lit), B In key, C Chord pads and D In key an octave up. A chord pad plays three or four notes. SHIFT + wheel on the Play page moves the pads by an octave, from C0 to C6 at pad 1 (with a drum kit it still moves the kit). With no scale the pads light C in purple, the other white keys in light blue and the black keys in dark blue; with a scale the root (or the home chord) is purple, the scale's notes light blue and the rest dark blue, all colours Akai's app was seen sending. The screen labels each pad with its note, or its chord ("iv7 Dm7"), and the header chip reads "FM · D Dorian · Chord pads · D3 to D4" (or "Poly · C3 to D#4" with no scale).
- **Drums.** A kit, not a keyboard: pad 1 plays the kit's Pad 1 note (on the Synth tab's Kit page, C2 by default) and each pad after it a semitone higher, whatever the scale, layout, bank or octave; SHIFT + wheel does nothing. Each pad is labelled with its sound ("Kick", "Closed hat") and lit in the kit's colour, the chip reads "Drums · Boom Kit", and the Synth tab shows the page of the pad hit last (`mpfree_synth_ui::pages::drum_page`). A new Pad 1 note, a pad's sound or another kit sends the pad grid and the arp the new names and notes. Pluck and Wavetable play the pad layouts as Poly and FM do. Neither takes pad pressure yet.
- **The layout owner's part.** `App::synth_toggle_layout` (the quick switch, Chromatic to In key and back) and `App::synth_bank` (a bank's layout) hand the synth new settings; which button or touch does each is the Hello MPC thread's to place.
- **Playing.** Note Repeat, Latch, Full and Half Level and the velocity curve work as for drums: the note engine hands the synth each note-on and note-off on its exact frame. Pad pressure goes to every note its pad plays. Two pads can share a note (two chords): a shared note takes the harder press and ends only when the last pad holding it is let go.
- **The Synth tab.** It sits between Clips and Looper. Q-Links 1 to 4 turn the page's marked knobs while it shows; elsewhere Q-Link 1 is still the level. The wheel turns the selected knob. SHIFT + wheel, - and + step the pages. A touch, Q-Link or wheel change goes to the audio thread and applies from the next period. After a preset load the audio thread sends the node's values back; every 250 ms it sends the meters and each voice's pressure (for the Pressure page's live bars) when they change.
- **Sound.** Each period the sampler and the synth render into their own tracks of the pad player's mixer ([`mixing.rs`](../../../rust/mpfree-app/src/mixing.rs)), whose master fader is the Level and whose Master safety keeps the output under full scale. Every engine plays on the synth's track. An engine not on the pads renders only while a note of its own still rings. The synth's render uses fixed buffers and never allocates (a test watches it with a counting allocator, through engine, scale, bank and tuning changes). A block with a non-finite sample is dropped, that node reset and the reset counted. STOP fades every engine out over 10 ms and resets them, so no release rings on.
- **The log.** At the start, `synth: Poly, preset Pluck, as kit list entry N (C3 to D#4)`. Then a line when the pads move to the synth, by an octave or to another layout (`pads: FM synth, D Dorian, Chord pads (bank A), D3 to D4 (pad 1 = D3; …)`), one on an engine switch (`synth: FM on the pads, preset Glass Keys`), one per preset (`synth: preset 4 of 9, Supersaw`) and one per knob once it rests for a second (`synth: Filter Cutoff 2.5 kHz`). Every 30 s a second status line gives the synth's engine, render time per period (mean and largest, also as a share of a period), notes, the most voices at once, notes dropped and resets. The sampler's line times the sampler alone.

## Next

- Tunings and expression: Tone routing in Poly, FM and Wavetable (two slots on the pressure targets, on the Pressure page), a tuning per track slot. Then Q-Link banks, the sampler as a node and the instrument browser (plan doc "Four synth features: plan").
- Presets as files: the format is built ([Sound presets](presets.md)); next, the catalogue and Save and Load with the browser page.
- A track host: built in `rust/mpfree-track` ([Track host](track-host.md)); next, wiring it into the pad player.
- The Sample tab's "Make a wavetable": see [Wavetable synth](wavetable.md).
- Plugin hosting: the CLAP host and its sandbox process are built ([Plugins](plugins.md)); next, ARM builds of free plugins (P3).
