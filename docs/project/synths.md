# Synths: the node interface and the Poly synth

**Status:** 2026-11-11, host-only and tested. The code is [`rust/mpfree-node`](../../rust/mpfree-node) (the interface every instrument and effect uses) and [`rust/mpfree-synth`](../../rust/mpfree-synth) (native synths; Poly first). Nothing here is wired to the pad player or has run on the MPC. The full design, the roadmap and the mockups are in the design doc "MPFree synths and plugins: design" and in mpfree-claude (`synths/mockups/`, `skill-tree/synth-skill-tree.html`).

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

## Poly (`mpfree-synth`)

A subtractive synth:

- **Oscillators:** two per voice (saw and pulse band-limited with PolyBLEP, triangle, sine), octave, semitone and fine tune, pulse width; a square sub one octave down; white noise.
- **Unison:** 1 to 4 stacks per voice, detuned symmetrically and spread across the stereo field. Each voice has a left and a right filter, so the spread stays stereo.
- **Drive and filter:** a `tanh` drive (0 dB is fully clean) into a zero-delay-feedback ladder (LP 24) or a trapezoidal state-variable filter (LP 12, BP 12, HP 12). Cutoff follows the filter envelope, key tracking, velocity, the LFO and pad pressure.
- **Envelopes:** filter and amp ADSR with exponential segments; a retrigger starts from the current level, so it never clicks.
- **LFO:** sine, triangle, saw, square or sample-and-hold, free (0.05 to 30 Hz) or synced to the transport (4 bars to 1/16 triplets, phase-locked to `Ctx::beats` while playing), to pitch, cutoff, pulse width and level.
- **Voices:** poly (1 to 16), mono or legato, glide, bend range. A stolen voice fades out over 3 ms in one of 4 spare slots. Stealing takes the quietest releasing voice, then the oldest held one. Mono and legato keep a note stack.
- **Presets:** Init, Grit Bass, Acid, Supersaw, Dust Pad, Pluck, Brass, Sub Bass and Lead.

Timing: oscillators, drive, filters, the amp envelope and the steal fade run every frame. Smoothing, the LFO, the filter envelope, glide, pitch and the filter coefficients run once every 16 frames, on frames divisible by 16.

## Tests

- `mpfree-node`: curves round-trip, clamping, description checks, event splitting, smoothing.
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
- `tests/no_alloc.rs`: a counting allocator watches 4000 blocks of random notes, pressure, bend, parameter changes over every parameter, AllOff and a reset, and sees no allocation or free.
- `dsp` unit tests: oscillator bounds, envelope stages, low-pass and high-pass responses.

## Listening and timing

- `cargo run --release -p mpfree-synth --example render [out.wav]` writes `~/mpfree-work/synth-demo.wav`: 9 bars at 92 BPM with synthesised drums, where Grit Bass, Acid (a cutoff sweep by automation), Dust Pad, Supersaw and Lead come in one after another, then a final chord rings out. It prints a timeline, each part's level per bar, checks each part sounds only where it should, and trims the master to -1 dBFS. The parts run through `Node::process` in 128-frame blocks.
- `cargo run --release -p mpfree-synth --example bench` times one 128-frame block with 1 to 16 voices and unison 1 or 4. On the build host (an x86 desktop core) 16 voices take 3.6 % of real time at unison 1 and 5.6 % at unison 4. The MPC One's Cortex-A17 is slower; it needs its own run before voice limits are set.

## Next

- A track host: a track's notes from `mpfree-play` and the pads into its instrument node, then its `mpfree-fx` chain.
- FM (6 operators, DX7 banks), then a drum synth for the pads.
- The Synth tab (`mpfree-synth-ui`) on mpfree-ui, now that PR #31 has merged.
- Plugin hosting: CLAP first, in a sandbox process.
