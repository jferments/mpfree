# Synths: the node interface, Poly and FM

**Status:** 2026-11-11, host-only and tested. The code is [`rust/mpfree-node`](../../rust/mpfree-node) (the interface every instrument and effect uses) and [`rust/mpfree-synth`](../../rust/mpfree-synth) (native synths: Poly, then FM). Nothing here is wired to the pad player or has run on the MPC. The full design, the roadmap and the mockups are in the design doc "MPFree synths and plugins: design" and in mpfree-claude (`synths/mockups/`, `skill-tree/synth-skill-tree.html`).

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

## FM (`mpfree-synth`, `Fm`)

A six-operator FM synth in the DX7's mould. It is DX7-style, not a bit-exact clone: the numbers come from published descriptions of the DX7 (the operator level table, the envelope rate formula, the keyboard scaling curves), and where the original is not public (the LFO speed curve, the pitch envelope's tables, detune) the values are close approximations.

- **Operators:** six sines, each with a ratio (coarse 0 to 31, fine) or a fixed frequency, detune, an output level (0 to 99), a four-rate, four-level envelope, velocity sensitivity, keyboard level scaling (break point, left and right depth and curve), rate scaling and amplitude modulation from the LFO. Each operator can be switched off.
- **Algorithms:** the DX7's 32 ways of wiring them, with feedback 0 to 7 on one operator or, for algorithms 4 and 6, around a loop of two or three. Feedback averages the last two samples, as the DX7 does, so it stays stable at 7.
- **Envelopes:** in decibels, falling at a constant rate, with the attack curving over as it nears its level. Rates and levels from 0 to 99 mean what they mean on a DX7.
- **Global:** a pitch envelope, an LFO (triangle, saw down, saw up, square, sine, sample-and-hold; speed, delay, pitch and amplitude depth, key sync), transpose, oscillator key sync, poly (1 to 16) or mono with a note stack, bend range, pad pressure for extra vibrato, and an output level.
- **Level:** one carrier at full level is about -12 dBFS. A voice with more carriers is scaled by 1/sqrt(carriers), so changing the algorithm doesn't jump the volume. The output level glides while notes sound and jumps straight to a new value when nothing sounds, so loading a preset never starts with a swell.
- **Voices:** a stolen voice fades out over 3 ms in one of 4 spare slots, like Poly.
- **Presets:** Init (one sine), Glass Keys, Bell Tower, Rubber Bass, Drawbars, Brass Section, Log Drum, Silk Pad and Steel Pluck. Each plays a chord between -11 and -6 dBFS peak.
- **DX7 banks** (`dx7`): reads a player's own .syx files, a 32-voice bank (4104 bytes) or a single voice (163 bytes), checks the checksum and maps every byte onto FM parameters; it can also write a bank back. Names are 10 characters, as on a DX7, and the bank has no output level, so imported voices play at 0 dB. Only voice data is read; nothing is sent to any device. MPFree ships no Yamaha voices.

Timing: operators run every frame. Envelopes, pitch and the LFO run once every 16 frames, on frames divisible by 16, with operator gains ramped across each step.

**On the Synth tab** FM has its own pages: Algo (algorithm, feedback, transpose, level, voices), Mod (LFO and pitch envelope), Op 1 to Op 6 (each operator's frequency, level, envelope and velocity; the Q-Links turn output, coarse, fine and velocity) and Kbd 1-2, 3-4 and 5-6 (keyboard scaling, two operators a page). Every FM parameter is on exactly one page. `cargo run --locked -p mpfree-synth-ui --bin mpfree-synth-preview` draws the samples `synth-fm-algo`, `synth-fm-op` and `synth-fm-kbd`. FM isn't in the pad player yet; Poly is the pads' synth for round 6.

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
- `tests/no_alloc.rs`: a counting allocator watches 4000 blocks of random notes, pressure, bend, parameter changes over every parameter, AllOff and a reset, and sees no allocation or free. It runs Poly and FM.
- `dsp` unit tests: oscillator bounds, envelope stages, low-pass and high-pass responses.

## Listening and timing

- `cargo run --release -p mpfree-synth --example render [out.wav]` writes `~/mpfree-work/synth-demo.wav`: 9 bars at 92 BPM with synthesised drums, where Grit Bass, Acid (a cutoff sweep by automation), Dust Pad, Supersaw and Lead come in one after another, then a final chord rings out. It prints a timeline, each part's level per bar, checks each part sounds only where it should, and trims the master to -1 dBFS. The parts run through `Node::process` in 128-frame blocks.
- `cargo run --release -p mpfree-synth --example fm_render [out.wav]` writes `~/mpfree-work/fm-demo.wav`: 9 bars at 92 BPM with the same drums. Rubber Bass plays all the way through; Log Drum comes in for bars 3 and 4 (velocity drives its modulator); Glass Keys chords and Brass Section stabs (algorithm 22 and its pitch envelope) take bars 5 and 6; Bell Tower (with a bend down a semitone and back) and Steel Pluck, read back from a DX7 bank written from the presets, take bars 7 and 8; the final chord rings out. It checks the same things as the Poly demo, plus that the end is quiet and no voice is left on.
- `cargo run --release -p mpfree-synth --example bench` times one 128-frame block with 1 to 16 voices: Poly at unison 1 or 4, and FM's Brass Section and Glass Keys. On the build host (an x86 desktop core) 16 voices take 3.6 % of real time for Poly at unison 1, 5.6 % at unison 4, and about 2.5 % for FM. The MPC One's Cortex-A17 is slower; it needs its own run before voice limits are set.

## The Synth tab (`rust/mpfree-synth-ui`)

The instrument page for the MPC One's 1280x800 screen, on `mpfree-ui`. It works for any `Node`, so hosted plugins and effects get the same page.

- **Pages.** A native engine names its pages and what its Q-Links turn (Poly: Osc, Filter, Env, LFO, Voice; FM: see above). Any other node gets pages from its `ParamInfo` groups: consecutive groups share a page up to 12 tiles, and a group bigger than 16 is split. Every parameter is on exactly one page.
- **Tiles.** Continuous and whole-number parameters are knobs; a bipolar one fills from the top. Choices of up to four show as buttons when they fit, otherwise as the chosen name with arrows. Toggles are switches. Read-only parameters are live meters. A tile's label carries its group when the group isn't the page's ("Env · Attack" on the Filter page).
- **Visuals.** Poly gets four cards drawn from its values: both oscillators, the filter response (with a dashed line where the filter envelope takes it), the amp and filter envelopes, and the LFO. A tap on a card opens its page. Other nodes get the plain layout without cards.
- **Control.**
  - A finger dragged up or down a knob turns it; 320 px is the whole range.
  - A tap selects a tile, and the wheel then turns it.
  - Q-Links 1 to 4 turn the marked tiles. A detent is one step for whole numbers and choices, and 1/100 of the range along the parameter's curve otherwise.
  - SHIFT + wheel steps through the pages.
- **The app's part.** The app owns the node on the audio thread. It builds a `SynthState` from the node (`SynthState::from_node`) and adds the panel with `SynthPanel::add(ui, "Synth", state)` after its own `header` and `hint` widgets. It passes touches (`SynthPanel::event`), Q-Link turns (`qlink`) and wheel turns (`wheel`, `step_page`) to the panel, and applies each `SynthAction` it gets back: `Set { id, value }` becomes `Node::set_param`, and `Preset(i)` loads the preset, after which the app calls `SynthPanel::load` with the node's new state. The panel updates its own copy of a value at once, so the screen keeps up with a finger.
- **Preview.** `cargo run --locked -p mpfree-synth-ui --bin mpfree-synth-preview` draws the sample screens to PNG files in a new folder `~/mpfree-work/synth-preview-<UTC>`. It opens no device.
- **Tests.**
  - The anti-aliased drawing, value text, page plans and visual curves (every curve stays in its box for every parameter at its minimum and maximum).
  - `tests/panel.rs` runs a Ui at 1280x800 with a real Poly. It checks that dragging, tapping choices and switches, Q-Links, the wheel and presets reach the node, that every Poly parameter can be reached and set, that the cards follow the page, that a node without a plan gets the plain page, and that every sample screen lays out inside the screen and redraws only the panel after a turn.

## In the pad player (`rust/mpfree-hello`, round 6)

The pad player (`play` and `app`) carries one Poly. `src/synth.rs` holds it on the audio thread; the Synth tab shows it.

- **Picking it.** The kit list ends with "Synth: Poly". Picking it with the wheel or a tap, or opening the Synth tab, hands the pads to Poly. Picking a drum kit hands them back, and Poly's notes end with their release. The sampler keeps its kit underneath, and its sounds finish.
- **Notes.** Pad 1 plays C3 and each pad after it a semitone higher, so the pads play C3 to D#4. SHIFT + wheel on the Play page moves them by an octave, from C0 to C6 at pad 1 (with a drum kit it still moves the kit). The pads light C in purple, the other white keys in light blue and the black keys in dark blue, all colours Akai's app was seen sending. The screen labels each pad with its note.
- **Playing.** Note Repeat, Latch, Full and Half Level and the velocity curve work as for drums: the note engine hands Poly each note-on and note-off on its exact frame. Pad pressure goes to the note its pad plays; Voice · Pressure opens its filter (1 octave at full pressure by default).
- **The Synth tab.** It sits between Play and Debug. Q-Links 1 to 4 turn the page's marked knobs while it shows; elsewhere Q-Link 1 is still the level. The wheel turns the selected knob. SHIFT + wheel, - and + step the pages. MENU opens the pad options, as on Play. A touch, Q-Link or wheel change goes to the audio thread and applies from the next period. After a preset load the audio thread sends the node's values back, and every 250 ms it sends the meters when they change.
- **Sound.** Each period the sampler renders, Poly adds to it, and the master level and the clip to full scale come last. Poly's render uses fixed buffers and never allocates (a test watches it with a counting allocator). A block with a non-finite sample is dropped, the node reset and the reset counted. STOP fades Poly out over 10 ms and resets it, so no release rings on.
- **The log.** At the start, `synth: Poly, preset Pluck, as kit list entry N (C3 to D#4)`. Then a line when the pads move to Poly or by an octave (`pads: Poly synth, C3 to D#4 …`), one per preset (`synth: preset 4 of 9, Supersaw`) and one per knob once it rests for a second (`synth: Filter Cutoff 2.5 kHz`). Every 30 s a second status line gives Poly's render time per period (mean and largest, also as a share of a period), notes, the most voices at once, notes dropped and resets. The sampler's line now times the sampler alone.

## Next

- A track host: a track's notes from `mpfree-play` and the pads into its instrument node, then its `mpfree-fx` chain.
- FM on the pads: an FM choice next to Poly in the pad player, and an algorithm diagram card on its Algo page.
- A drum synth for the pads.
- Plugin hosting: CLAP first, in a sandbox process.
