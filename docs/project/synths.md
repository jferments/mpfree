# Synths: the node interface, Poly, FM, Drums, Pluck, Wavetable and pad pressure

**Status:** 2026-11-11, host-only and tested. The code is [`rust/mpfree-node`](../../rust/mpfree-node) (the interface every instrument and effect uses) and [`rust/mpfree-synth`](../../rust/mpfree-synth) (native synths: Poly, FM, Drums, Pluck and Wavetable; Drums and Pluck merged in PR #97, Wavetable in PR #99). Poly plays on the pads (see "In the pad player"); FM, Drums, Pluck and Wavetable are not on the pads yet, and none has run its own cost bench on the MPC. The full design, the roadmap and the mockups are in the design doc "MPFree synths and plugins: design" and in mpfree-claude (`synths/mockups/`, `skill-tree/synth-skill-tree.html`).

## The node interface (`mpfree-node`)

One trait, `Node`, for anything that runs on the audio thread on a stereo bus: a synth, the sampler (later), a hosted plugin (later), an effect (the FX work builds `mpfree-fx` on it).

- `process(ctx, events, left, right)` once per block. `events` are `Timed { offset, ev }`, sorted, with offsets inside the block: `NoteOn`, `NoteOff`, `Pressure` (poly aftertouch), `Bend`, `AllOff` and `Param { id, value }` (automation, parameter locks, macro moves).
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

**On the Synth tab** Drums has a page per pad (the tab strip shows 1 to 16) and a Kit page. The Q-Links turn the pad's Tune, Decay, Tone and Snap. `pages::drum_page(pad)` gives the page of a pad, so the app can follow the last pad hit. The preview draws `synth-drums-kick`, `synth-drums-boom` and `synth-drums-hat`. `Drums::pad_name(pad)` gives the model name for pad labels.

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

## Wavetable (`mpfree-synth`, `Wavetable`)

Two table oscillators that scan through stacks of single-cycle frames, with warps (Bend, Sync, Mirror, FM), unison, a sub and noise, Poly's filter, a mod envelope and LFO that move the positions, and pad pressure. Ten factory tables are built from formulas; a player's WAV tables and a pad's sample can become user tables. Tables are band-limited at 17 half-octave levels, so high notes don't alias. The full description, the memory and cost numbers and the demo are in [Wavetable synth](wavetable.md).

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
- `tests/no_alloc.rs`: a counting allocator watches 4000 blocks of random notes, pressure, bend, parameter changes over every parameter, AllOff and a reset, and sees no allocation or free. It runs Poly and FM.
- `dsp` unit tests: oscillator bounds, envelope stages, low-pass and high-pass responses.

## Listening and timing

- `cargo run --release -p mpfree-synth --example render [out.wav]` writes `~/mpfree-work/synth-demo.wav`: 9 bars at 92 BPM with synthesised drums, where Grit Bass, Acid (a cutoff sweep by automation), Dust Pad, Supersaw and Lead come in one after another, then a final chord rings out. It prints a timeline, each part's level per bar, checks each part sounds only where it should, and trims the master to -1 dBFS. The parts run through `Node::process` in 128-frame blocks.
- `cargo run --release -p mpfree-synth --example fm_render [out.wav]` writes `~/mpfree-work/fm-demo.wav`: 9 bars at 92 BPM with the same drums. Rubber Bass plays all the way through; Log Drum comes in for bars 3 and 4 (velocity drives its modulator); Glass Keys chords and Brass Section stabs (algorithm 22 and its pitch envelope) take bars 5 and 6; Bell Tower (with a bend down a semitone and back) and Steel Pluck, read back from a DX7 bank written from the presets, take bars 7 and 8; the final chord rings out. It checks the same things as the Poly demo, plus that the end is quiet and no voice is left on.
- `cargo run --release -p mpfree-synth --example pressure_render [out.wav]` writes `~/mpfree-work/pressure-demo.wav`: the same 4-bar phrase twice at 90 BPM with the same notes and the same pad pressure, first with pressure Off on every synth, then on. A Brass chord with only its top note pressed (it swells twice), an Acid line that opens as the phrase builds, an FM Brass Section chord that swells and brightens, and a Dust Pad with vibrato on its top note; a last Brass chord swells and rings out in bar 9. It measures what pressure does in the second pass (the level difference between the passes, and brightness), checks each part clearly moves, and trims the master to -1 dBFS.
- `cargo run --release -p mpfree-synth --example drum_pluck_render [out.wav]` writes `~/mpfree-work/drum-pluck-demo.wav`: 11 bars at 90 BPM. Bars 1-2 are the Init Kit alone (the open hat choked by the next closed hat, a tom fill); then the Grit Kit with the Muted Bass pluck; the Boom Kit with a Koto line; the Electro Kit with a Marimba riff; the Grit Kit with a Bell melody and a Kalimba arpeggio; and a Glass chord and a last boom ring out. Kits change on the bar line while the last hits ring on. It prints a timeline and each part's level per bar, checks each part sounds only where it should and the end is quiet, and trims the master to -1 dBFS.
- `mpfree-bench` has cases for both (module "synth"): Drums Init Kit with a hit every 4 blocks and Boom Kit with a hit every block, and Pluck Steel String (8 voices), Harp (16), Marimba (8) and Bell (16) with chords changing every half second. On the build host (an x86 server core, 44.1 kHz, 128 frames) Drums take about 2 % of a period at p99 and the plucks 0.8 to 1.4 %; the MPC One's numbers need its own run (`--calibration`).
- `cargo run --release -p mpfree-synth --example bench` times one 128-frame block with 1 to 16 voices: Poly at unison 1 or 4, and FM's Brass Section and Glass Keys. On the build host (an x86 desktop core) 16 voices take about 4 % of real time for Poly at unison 1, 5 to 6 % at unison 4, and about 3 % for FM; this host's runs vary by up to a third between runs, and pressure made no difference beyond that. The MPC One's Cortex-A17 is slower; it needs its own run before voice limits are set.

## The Synth tab (`rust/mpfree-synth-ui`)

The instrument page for the MPC One's 1280x800 screen, on `mpfree-ui`. It works for any `Node`, so hosted plugins and effects get the same page.

- **Pages.** A native engine names its pages and what its Q-Links turn (Poly: Osc, Filter, Env, LFO, Voice, Pressure; FM: see above). Any other node gets pages from its `ParamInfo` groups: consecutive groups share a page up to 12 tiles, and a group bigger than 16 is split. Every parameter is on exactly one page.
- **Tiles.** Continuous and whole-number parameters are knobs; a bipolar one fills from the top. Choices of up to four show as buttons when they fit, otherwise as the chosen name with arrows. Toggles are switches. Read-only parameters are live meters. A tile's label carries its group when the group isn't the page's ("Env · Attack" on the Filter page).
- **Visuals.** Poly gets four cards drawn from its values: both oscillators, the filter response (with a dashed line where the filter envelope takes it), the amp and filter envelopes, and the LFO. A tap on a card opens its page. FM gets the algorithm diagram and a card for each page (see above). On the Pressure page (Poly and FM) the strip shows the pressure curve and the live bars instead. Other nodes get the plain layout without cards.
- **Control.**
  - A finger dragged up or down a knob turns it; 320 px is the whole range.
  - A tap selects a tile, and the wheel then turns it.
  - Q-Links 1 to 4 turn the marked tiles. A detent is one step for whole numbers and choices, and 1/100 of the range along the parameter's curve otherwise.
  - SHIFT + wheel steps through the pages.
- **The app's part.** The app owns the node on the audio thread. It builds a `SynthState` from the node (`SynthState::from_node`) and adds the panel with `SynthPanel::add(ui, "Synth", state)` after its own `header` and `hint` widgets. It passes touches (`SynthPanel::event`), Q-Link turns (`qlink`) and wheel turns (`wheel`, `step_page`) to the panel, and applies each `SynthAction` it gets back: `Set { id, value }` becomes `Node::set_param`, and `Preset(i)` loads the preset, after which the app calls `SynthPanel::load` with the node's new state. The panel updates its own copy of a value at once, so the screen keeps up with a finger.
- **Preview.** `cargo run --locked -p mpfree-synth-ui --bin mpfree-synth-preview` draws the sample screens to PNG files in a new folder `~/mpfree-work/synth-preview-<UTC>`. It opens no device.
- **Tests.**
  - The anti-aliased drawing, value text, page plans and visual curves (every curve stays in its box for every parameter at its minimum and maximum). All 32 algorithms lay out with no boxes overlapping, carriers on the bottom row and every modulator above what it modulates.
  - `tests/panel.rs` runs a Ui at 1280x800 with a real Poly. It checks that dragging, tapping choices and switches, Q-Links, the wheel and presets reach the node, that every Poly parameter can be reached and set, that the cards follow the page, that the Pressure page draws the curve and live bars and reads depths in their targets' units, that FM draws its algorithm, rings the page's operators and opens an operator's page from a tap on its box, that every FM page has its strip, that a node without a plan gets the plain page, and that every sample screen lays out inside the screen and redraws only the panel after a turn.

## In the pad player (`rust/mpfree-hello`, round 6)

The pad player (`play` and `app`) carries one Poly. `src/synth.rs` holds it on the audio thread; the Synth tab shows it.

- **Picking it.** The kit list ends with "Synth: Poly". Picking it with the wheel or a tap, or opening the Synth tab, hands the pads to Poly. Picking a drum kit hands them back, and Poly's notes end with their release. The sampler keeps its kit underneath, and its sounds finish.
- **Notes.** Pad 1 plays C3 and each pad after it a semitone higher, so the pads play C3 to D#4. SHIFT + wheel on the Play page moves them by an octave, from C0 to C6 at pad 1 (with a drum kit it still moves the kit). The pads light C in purple, the other white keys in light blue and the black keys in dark blue, all colours Akai's app was seen sending. The screen labels each pad with its note.
- **Playing.** Note Repeat, Latch, Full and Half Level and the velocity curve work as for drums: the note engine hands Poly each note-on and note-off on its exact frame. Pad pressure goes to the note its pad plays; Voice · Pressure opens its filter (1 octave at full pressure by default).
- **The Synth tab.** It sits between Play and Debug. Q-Links 1 to 4 turn the page's marked knobs while it shows; elsewhere Q-Link 1 is still the level. The wheel turns the selected knob. SHIFT + wheel, - and + step the pages. MENU opens the pad options, as on Play. A touch, Q-Link or wheel change goes to the audio thread and applies from the next period. After a preset load the audio thread sends the node's values back, and every 250 ms it sends the meters when they change.
- **Sound.** Each period the sampler renders, Poly adds to it, and the master level and the clip to full scale come last. Poly's render uses fixed buffers and never allocates (a test watches it with a counting allocator). A block with a non-finite sample is dropped, the node reset and the reset counted. STOP fades Poly out over 10 ms and resets it, so no release rings on.
- **The log.** At the start, `synth: Poly, preset Pluck, as kit list entry N (C3 to D#4)`. Then a line when the pads move to Poly or by an octave (`pads: Poly synth, C3 to D#4 …`), one per preset (`synth: preset 4 of 9, Supersaw`) and one per knob once it rests for a second (`synth: Filter Cutoff 2.5 kHz`). Every 30 s a second status line gives Poly's render time per period (mean and largest, also as a share of a period), notes, the most voices at once, notes dropped and resets. The sampler's line now times the sampler alone.

## Next

- A track host: built in `rust/mpfree-track` ([Track host](track-host.md)); next, wiring it into the pad player.
- FM on the pads: an FM choice next to Poly in the pad player.
- Wavetable on the pads (a third Engine choice) and the Sample tab's "Make a wavetable": see [Wavetable synth](wavetable.md).
- Drums and Pluck in the pad player: Drums as an engine that sends each pad its own note (no pad layout or octave), with model names on the pads; Pluck through the pad layouts like Poly and FM.
- Synth tab visuals for Drums and Pluck (a pad grid with the models, the string or the mode bank).
- Plugin hosting: the CLAP host and its sandbox process are built ([Plugins](plugins.md)); next, ARM builds of free plugins (P3).
