# Synth library: architecture and roadmap

**Status:** design, 2026-11-18 (synth library stage 3), now being built: the voice core's part 1 and the controller event (PR #579), the Ensemble (PR #591), Strings and Bass (wave 1) are in; the roadmap at the end names the pull request each other part comes in. It rests on the research pages ([What every type shares](../research/synth-topics.md) and the type pages it links) and on the measured baseline ([Synth quality](synth-quality.md)). Plan: "Synth library: stock-take and research plan" (https://claude.ai/artifact/36gY5Lf3qTVZp21RM9pzUp). **Nothing here has run on the MPC**: every cost is the workstation's or an estimate, and voice counts wait for the device round asked for in the project files (`synth-library/device-round-asks.md`).

Where a point is our own reading or estimate, it says so. Changes to the shared screen layout are only proposals here ([layout spec](layout-spec.md)); this page needs none for wave 1.

## 1. What the baseline says to fix first

From [Synth quality](synth-quality.md), the faults every engine shares, in the order they matter to a player:

| Fault | Measured | Fixed by |
|---|---|---|
| Aliasing on classic waves | Poly's saw −37 dB at C7, Analog's −42 dB; 9 of 36 cases meet −60 dB. Wavetable's saw table reaches −72 dB at C7, for less work a voice (3.5 µs against Poly's 4.1 on the workstation) | Classic waves read from band-limited tables ([section 3](#3-the-shared-voice-core)) |
| Aliasing in drive and distortion | Poly's saw through 24 dB of drive −30 dB at C7 | Antiderivative antialiasing on the drive's curve, or 2x around it, whichever measures better |
| Velocity barely changes the level | Poly 7 to 8 dB from velocity 20 to 127, against 30 to 40 dB for keys | A square-law curve with a range per preset |
| Velocity doesn't change the tone | 32 of 71 presets | Velocity routes on every engine |
| Chords clip | 17 presets clip an 8-note chord | The preset rules and checks ([section 5](#5-what-a-preset-carries)) |
| Labels that don't say what happens | Poly's Attack reaches full level at 31 % of its label, Release at 85 % | Envelope times that mean what they say |
| DC | 8 presets above −60 dBFS (FM's 1:1 pairs, reeds) | A DC blocker at every engine's output |
| No sustain pedal, mod wheel or breath | Instruments hear only notes, pressure, bend, per-note expression and parameters | A controller event ([section 6](#6-how-each-instrument-is-played)) |

## 2. The engines, by type

Twelve engines when the three waves are done, five of them new or merged, and two shared parts. "Second algorithm" is offered only where it sounds clearly different and players ask for it (the plan's rule).

| Type | Engine | Algorithm | Second algorithm | Wave |
|---|---|---|---|---|
| Bass and 808 | **Bass** (new, one voice, built 18 Nov: [synths.md](synths.md#bass-mpfree-synth-bass)) | Synth body: an oscillator (saw, pulse, triangle, sine), a sub locked to it (a square one or two octaves down, a narrow pulse two down, or a sine), a Fifth, noise, into the ladder, then drive with a low-pass after it. 808 body: a sine or triangle with a pitch drop (Punch, up to an octave for 6 ms; Drop and Drop time), the amp envelope, drive and the low-pass. Both settle exactly on the note. Not built: the 808's optional pitch wander | Reese: two detuned oscillators over a clean sub, a body of its own | 1 |
| Acid | Squelch (as built) | Diode ladder, accent, slide | | |
| Electric piano and keys | **Keys** (extended; the clav built 18 Nov: [synths.md](synths.md#keys-mpfree-synth-keys)) | Tine and reed (built) | Clav: a waveguide string struck by a shaped pulse, the pickup as a comb and a curve, a pitch drop at release (Gabrielli et al.). Electric grand later | 1 |
| String machine | **Strings** (new, built 18 Nov: [synths.md](synths.md#strings-mpfree-synth-strings)) | A bank with a band-limited saw for every note, each octave's phase locked to twice the octave below as when the top octave is divided down; only the notes heard are rendered. Registers (violin, viola, cello, contrabass, horn, trumpet) as mixes of three footages through fixed tone filters on shared buses; a swell per key or shared; bow noise. The **Ensemble** inside it | | 1 |
| Ensemble | **Ensemble** (new, in `mpfree-fx`) | Three bucket-brigade lines, each moved by a slow and a fast LFO 120° apart (0.66 and 6.25 Hz), or four lines in the later Roland pattern; the line is Analog's chorus line | | 1 |
| Poly and wavetable | **Poly** (Wavetable folded in) | Per oscillator: Classic (from tables), Table (today's Wavetable) or Supersaw (seven saws on the detune curve and mix); the ladder and state-variable filters as today; a second LFO | | 2 |
| Vintage poly | Analog (as built) | Voice cards that differ and drift, the OTA filter, the bucket-brigade chorus | | |
| FM | FM (fixed) | As built, plus macros, a few operator shapes besides the sine, DC blocking, level matching across algorithms, and feedback that aliases less at high notes | | 2 |
| Organ | Drawbar (+ a Combo model) | Tonewheel (built); combo organ from the Strings bank with flute and reed voice filters | Pipe organ later | 2 |
| Wind | **Wind** (new) | A waveguide bore excited by a reed or jet curve, driven by breath (pressure or the breath controller) | FM and subtractive winds as FM and Poly presets | 2 |
| Choir | **Choir** (new) | A buzzy source through five formant filters per voice, morphing between vowels, through the Ensemble | Wavetable's Vowels table as Poly presets | 2 |
| Tape character | **Character** (shared part) | Per-voice offsets and slow drift, wow and flutter, a note time limit, hiss; off and bit-exact at zero | | 2 |
| Drums | Drums (extended) | As built, plus each drum's own knobs, each drum's own two effects in a kit, accent and per-hit variation, the 808 body shared with Bass, toms and congas from Pluck's modal bank | | 3 |
| Plucks and mallets | Pluck (polished) | As built; the comb notches of Position fixed | | 3 |
| Expressive leads | Poly, Analog and Bass presets | Velocity and play routes, per-note expression, note priority | | 3 |

Why these choices (our reading of the research pages):

- **Bass is its own engine**, not Poly in mono: its parts (a dividing sub, the Fifth, note priority, a glide at a constant rate, the 808 body, a low-pass after the drive) are what make it a bass, and it costs one voice. It is built from the shared voice core, so it is mostly assembly.
- **Strings is the cheapest rich sound in the library.** Every key can sound, as on the string machines. Planned as a bank whose cost is flat with keys held; as built (18 Nov) it renders only the notes heard, so its cost grows with the notes, but slowly: the filters run on shared buses, not per key, and on the workstation one key costs 154k instructions a block (most of it the Ensemble) and 49 keys about 2.5 times that. A track at rest stops running. The same bank later gives the combo organ.
- **Tunings and the bank.** Locking keeps octaves exact, so in any tuning whose octaves are pure (12-TET at any A4, Just, Pythagorean, meantone, a 12-note Scala file) each octave is locked to the one below and keys an octave apart never beat. In any other tuning (24-TET, a scale of another size, a stretched octave) each note runs free at its own pitch, at the same cost, so every key is still in tune (the research's S6). As built, nothing on the Synth tab needs to say which.
- **The Ensemble is an effect**, so any track can use it: a Poly pad or a choir through it is half of the string-machine sound.
- **Keys gets the clav, not a piano.** A full modelled grand is the heaviest instrument in the research and doesn't fit the budget (Bank et al.); FM pianos stay in FM, and a light electric grand comes after wave 3 if the device numbers leave room.

## 3. The shared voice core

Every engine re-implements parts of the same voice. The core is a set of plain parts in `mpfree-synth` (`voice.rs` and `voice/`), each with its own tests, that engines assemble. It is not a rewrite: **new engines (Bass, Strings, Wind, Choir) are built on it, and an existing engine moves onto a part when that part fixes something measured.** A move that should not change the sound is checked bit for bit against the old engine; one that should is checked against the ratchet and the blind listening.

| Part | What it does | Today |
|---|---|---|
| **Notes** | The note stack and play mode: poly, mono, legato, and (Strings, organ) every key. Note priority: last, low or high. Sustain and sostenuto: a note let go while the pedal is down keeps sounding until the pedal comes up; a key struck again under the pedal restarts its voice | Each engine has its own stack; no priority choice; no pedal |
| **Voices** | Which voice a note gets: a free one, the same note's own, then a stolen one, taking released voices first and the quietest of those, then the oldest held. A stolen voice fades over 3 ms in a spare slot. Counts what it steals | Poly, FM and Wavetable each have a copy; the 3 ms fade is shared by habit |
| **Glide** | Constant time or constant rate (semitones a second), on legato only or always | One glide time; constant time only |
| **Envelopes** | Exponential segments that retrigger from where they are; **a time means the time to get there** (an attack of 100 ms reaches full level at 100 ms) | `dsp::Env`: Attack aims past full level and reaches it at 31 % of its label |
| **LFOs** | Free or synced, a delay and fade-in, key retrigger, one per engine and two per voice | One per engine |
| **Velocity** | A square-law curve over a range in dB set per preset (Dannenberg; 30 to 40 dB for keys, less for pads), and routes to tone | Fixed 7 to 8 dB on Poly |
| **Play routes** | Eight routes a sound: a source (velocity, pressure, per-note tone, mod wheel, breath, foot controller, key, random per note) to a target from the engine's list, with a depth. Pressure's four slots become routes with pressure as their source; old presets load into them | Pressure only, four slots |
| **Oscillators** | Band-limited tables with levels half an octave apart (Wavetable's), including the classic waves, a pulse of any width as two saws, and the supersaw; PolyBLEP kept where it is cheaper and good enough (sub, drums) | PolyBLEP in Poly and Analog; tables in Wavetable |
| **Filters and drive** | The ladder, state-variable, OTA and diode filters as they are; the drive's curve with antiderivative antialiasing, or run at twice the rate, by measurement | Drive at the base rate |
| **Output** | A DC blocker, the level correction for drive, the 3 ms end fade | DC passes through on some presets |
| **Character** | Seeded per-voice offsets and drift (Analog's voice cards), wow and flutter, labelled amounts; at zero, bit-exact | Analog only |

**Part 1 is built** (`mpfree-synth/src/voice.rs`, 22 unit tests): the note stack with priority, voices and stealing, glide by time or rate, the envelope whose times mean what they say, LFOs (free or synced, delay and fade-in, retrigger), the velocity curve, the play routes and the DC blocker. The pedals live in `mpfree-node` (`pedal::Pedals`), because the hosts use them too. No engine uses part 1 yet; Strings and Bass are the first. The oscillator tables, the drive and Character are part 2, with the engines that need them.

Rules every part keeps, as the node interface asks: no allocation, lock or panic while sounding; the same output for any block size; control work on absolute frames every 16 frames, as today.

**Laid out for speed.** In new engines, the values the inner loops touch are kept in rows across voices, four at a time, rather than one record per voice, so a NEON kernel can later replace a loop without changing the engine ([section 4](#4-voices-the-cpu-and-the-cores)). Existing engines change layout only when they move onto a part.

**Envelope times that mean what they say.** Changing Poly's attack curve would make every saved attack sound different. So the factory presets' times are rescaled in the same pull request so they sound the same, the setting gets a new name in preset files, and a preset or project saved with the old name is converted on load (the preset format's alias rule, [Presets](presets.md)). The listening gate checks that nothing changed.

## 4. Voices, the CPU and the cores

### Voices per engine

As accepted in the plan: Poly, Analog and Wavetable 8 by default and up to 16; FM 16; Keys 16; Strings and the organs every key; Bass, Squelch and Wind 1 (Wind may get a few for chords); Choir 8; Drums 32 hits. Each default waits for the device's per-voice cost (ask 1 of the device round), and "new engines must come with device numbers before they are turned on by default" ([Profiling](profiling.md#budgets-agreed-with-jay-2026-11-11)).

### Synth tracks on every sound core

Today every track's instrument renders one after another on the audio thread (`TrackHost::render`), and the synths together get 20 % of that one core in its worst period, 580 µs. Only the pad synth can move, to core 1, in the `three` test mode. The plan's lever 2 makes every synth track a job the sound cores share:

- **Groups on lanes.** The track host splits its slots into groups: one on the audio thread (core 3), one on core 2's lane, one on core 1's lane (in `three`). Each period it posts the lanes with their slots' events, renders its own group, joins the lanes by the period's one deadline, and adds every slot's output into its mixer track on the audio thread. A slot owns its instrument, buffers and event queue, so a group moves whole, and nothing is shared between cores ([Lanes](concurrency.md#lanes-rustmpfree-lanes)).
- **Placed by cost.** Each slot's probe already times its renders. Twice a second the host places the slots, the most costly first, on the core with the most room left; a slot moves only between periods, while every lane is home, and only when the gain is clear, so slots don't hop back and forth. A newly picked instrument starts where there is most room.
- **Late is counted, never silent without a trace.** A lane that misses the deadline leaves its slots out of that period, counted, logged and shown, like the sampler's half and the pad synth today.
- **Order within a period** (our reading of `cores.rs`): core 2's synth group runs in the same window as the sampler's half, and both are joined before the mixer's lanes start, so core 2's room for synths is what the sampler's half leaves; core 1 has only synths and the pad player's helpers.
- **Budgets per core.** "20 % of one core" becomes "each core's worst period under its line". Core 3 keeps the 60 % line for everything on it. The lanes' lines come from the device round (Sound cores A/B); until then the design assumes 60 % on cores 2 and 1 too.
- **Gain** (our estimate): about three times today's synth room in `three`, without any engine getting cheaper.

The pad synth becomes one more slot, so it is placed like any track.

### One heavy synth across two cores

A 16-voice pad can be more than one core's share. The voice core renders its voices in two ranges, the second on a lane, then sums them before the engine's own chorus or ensemble, as the sampler's `Engine::split` does. Built only once the per-track groups are in and the device shows a single sound passing its core's line.

### Rendering ahead (lever 4, sketched, not yet planned)

Tracks that only play back what the sequencer sends could render a few periods early on spare core time, so their worst periods are averaged out: the 14 Nov review's biggest lever, and its largest change. The rules it would keep:

- The armed track, and any track getting live input (pads, keys, a Q-Link on its instrument), plays on time, as now.
- A track that becomes live finishes the audio it has made ahead (a few periods, under 12 ms) and then plays on time; arming a track before playing it, as players do, hides this.
- STOP, a jump or a tempo change throws the ahead audio away and starts again; STOP already fades.
- The sequencer must give that track's notes and locks a few periods early, which it can for a song playing on its own.

It is built only if the device numbers show the per-core groups and flatter worst periods are not enough.

### Flatter worst periods (lever 5)

The worst period is two to three times the average on the device, and on the workstation some presets' worst block is 20 to 140 times their average (the `--afford` worst/mean column: Wavetable Sync Sweep, Squelch Screamer, Poly Pluck). Three fixes, measured one by one:

- **Note starts.** A bench case that starts 16 notes in one block finds each engine's start cost; work at a note's start (tables, coefficients, resets) is spread over its first control steps or done ahead.
- **Denormals.** The audio thread and every lane set the processor's flush-to-zero bit when they start (ARMv7's FPSCR, x86's MXCSR on the workstation), in one small audited place in the app; engines still flush their own tails so a silent engine puts out exact zeros. The sound changes by less than −700 dB.
- **The CPU clock.** A fixed clock while playing is a kernel request, in RAM only, on Jay's go after the read-only device look ([Profiling](profiling.md#the-stability-and-capacity-review-2026-11-14), item 7).

### NEON (lever 6): the decision

Stable Rust can't get float NEON code on the MPC from the compiler: LLVM vectorises float loops for ARMv7 NEON only when every operation is marked as allowed to be inexact ("fast"), because NEON flushes denormals ([LLVM r266363](https://lists.llvm.org/pipermail/llvm-commits/Week-of-Mon-20160411/347656.html)), and stable Rust has no such marking; the NEON intrinsics are nightly-only for 32-bit ARM ([Profiling, NEON](profiling.md#neon)). The options:

| Option | For | Against |
|---|---|---|
| **Inline assembly kernels** (`asm!`, stable Rust) in one small crate, `mpfree-simd`, each with a plain Rust twin | No new toolchain; the unsafe code stays in one place; the twin is the reference on the workstation | Hand-written assembly; each kernel needs a test under qemu-arm against its twin |
| C kernels with `arm_neon.h` through the `cc` crate | Intrinsics are easier to write than assembly | A C cross-compiler in every build and CI job |
| Nightly Rust | The real intrinsics | The whole project on a nightly compiler |
| Whole numbers (phase counters as `u32`) | LLVM does vectorise whole-number loops; dividers and phase counters are whole numbers anyway | Only part of the work |

**Decision:** inline assembly kernels with plain twins, plus whole-number phase counters, **once a bench shows the gain**. The first candidates are the supersaw's seven saws, the Strings bank's oscillators (one per note heard) and the Choir's five formants a voice. The bench runs the kernel and its twin under qemu-arm, checks they agree, and the device round times both. `mpfree-synth` keeps `#![forbid(unsafe_code)]`; only `mpfree-simd` holds unsafe code.

### Cheap by design (lever 7)

Each new engine is chosen for its cost: the Strings bank's filters and the Ensemble cost the same for one key or sixty, and only its oscillators grow with the notes heard; modulation runs every 16 frames; unison voices sit side by side for the kernels; modal banks keep few modes; silent voices are skipped; load shedding stays the safety net, never the budget.

## 5. What a preset carries

A preset today is the engine's settings, a name, tags and its Q-Link sets ([Presets](presets.md)). Players judge a sound with its effects and play it through its macros, so a preset grows into a **sound**:

| Part | What it is | In the file |
|---|---|---|
| Settings | Every setting of the engine, by name (as now) | `filter.cutoff = 1840` |
| Macros | Four to six named macros, each moving several settings along curves, from and to | `macro.1 = Brightness`, `macro.1.map = op-2.level 20 80 exp` and its position `macros.brightness = 35` |
| Effects | Up to three of the sound's own effects from `mpfree-fx`, in order, with their settings | `fx.1 = mpfree.fx.ensemble`, `fx-1.mix = 60` |
| Send hints | The reverb or delay level the sound was made for, used only when the track's send is still at its default | `send.a = -12` |
| Play | Voices, play mode, note priority, glide, velocity range, routes, the pad layout it suits (chord pads for strings) | `voice.count = 8`, `play.layout = chord` |
| Q-Link sets | As now | `qset = ...` |
| Category and tags | One category (Bass, Keys, Strings, Pad, Lead, Brass, Organ, Wind, Choir, Pluck, Drums, FX) and up to 8 tags | `category = Keys` |

**How it plays: the Sound wrapper.** A `Sound` (in `mpfree-instruments`) is a node that holds a native engine's node, its macros and its effects (a plugin keeps its own presets and isn't wrapped). To the track host, the pad synth, locks, the mod matrix, MIDI Learn and the Q-Links it is just a node with more settings: the macros and the effects' settings are parameters with ids from 61440 up, so a step lock on a macro or an LFO on the ensemble's depth works with no new code. Loading a sound never touches the track's mixer strip, whose inserts stay the player's own. The sound's effects cost CPU on the same core as the sound, and are placed with it ([section 4](#synth-tracks-on-every-sound-core)). Effects in a sound sleep on silence, as every effect does.

**Factory presets become files.** Today they are Rust tables (`params.rs`). They move to `.preset` text files in the repository, built into the program, so presets are written, reviewed and checked like any other file, and the automatic checks read every one: the level rule (a held chord within 3 LU of the others of its category), no clipping on an 8-note chord, DC under −60 dBFS, a velocity range and tone change, an even level across the keys, and its cost. This is stage 6's preset system, with the browser's Try, Load and Save wired.

**Morph** between two sounds of one engine (A to B on one knob) uses the mod engine's morph, which exists; it comes after the macros.

## 6. How each instrument is played

### A controller event

`mpfree-node` gets one more event, `NodeEvent::Control { ctl, value }`, for the controllers that belong to the whole instrument: sustain (CC 64), sostenuto (66), soft pedal (67), mod wheel (1), breath (2) and foot controller (11), each 0 to 1. MIDI in sends them to the armed track's instrument (the MIDI thread owns `midi_in.rs`, so the wiring is agreed with it); a plugin gets the wheel, breath and foot as its MIDI controllers, and the pedals as notes the host holds.

- An engine on the voice core handles sustain and sostenuto in its note stack, and routes the rest through its play routes.
- **An engine that doesn't hear the pedal yet still sustains**: the track host holds its note-offs while the pedal is down and sends them when it comes up. So the pedal works on every engine from the first pull request.
- Mod wheel, breath and the foot controller reach a sound through its routes (vibrato on the wheel, breath to a wind's pressure, the foot to a swell).

### On the MPC

| Type | The pads | Pressure | Q-Links (the sound's first set) |
|---|---|---|---|
| Bass | One note at a time; holding one pad and tapping the next glides (legato on overlap) | Drive or cutoff (growl), vibrato on long 808s | Cutoff, Env amount, Drive, Glide; 808s: Drop, Decay, Drive, Tone |
| Keys | Velocity tuned for pads with each preset's range | Tremolo depth, wah or drive | Tone, Bark, Tremolo, Effects |
| Strings | Chord pads | The swell under the finger, the ensemble's depth | Crescendo, Sustain, Ensemble, Brightness |
| Poly | Chord pads, the arp | Vibrato, cutoff, table position | The preset's macros |
| FM | Keys or chord pads | Brightness: every modulator at once | Brightness, Detune, Attack, Decay |
| Organ | Chord pads, every key | Rotary speed or drive | Drawbars, four at a time |
| Wind | One note, or a few | Breath: the note swells and opens | Breath, Vibrato, Tone, Noise |
| Choir | Chord pads | Swell and vowel | Vowel, Singers, Swell, Space |
| Drums | Each pad its drum | Open a hat, bend a tom | The shown drum's own four |

**Every sound's first page is its macros**, drawn as the sound moves (the filter, the table, the ensemble's lines), and the Q-Links turn them. The page is inside the Synth tab's frame, which the layout spec already fixes ([section 4g](layout-spec.md#4g-the-synth-tab)), so it needs no layout change.

**One later proposal for the layout owner:** a way to hold notes on the MPC itself for players with no pedal (which button, if any, is Jay's call). Nothing in wave 1 depends on it.

## 7. Roadmap

Three waves, each finished end to end: engine work, then presets, then the checks, blind listening and a device round. About two weeks in all (Jay, 18 Nov). Code pull requests go to the merge desk as drafts, with Jay's standing go; anything that touches the MPC waits for his go as always.

**Wave 1: Bass, Keys, Strings** (this week)

| # | Pull request | What it brings |
|---|---|---|
| 1 | Wave 1 targets and tests (stage 4) | The targets table for Bass, Keys and Strings with today's numbers, fxlab measures for the new parts, bench cases (note starts, the bank, the Bass voice) |
| 2 and 3 | Voice core, part 1, and the controller event (one pull request) | Notes with priority and pedals, voices and stealing, glide, envelopes, LFOs, velocity, play routes, DC blocker, with unit tests and no engine changes; `NodeEvent::Control` and the host's pedal fallback. MIDI in's controllers follow in the MIDI thread's pull request |
| 4 | Ensemble effect | In `mpfree-fx`, measured the FX way. Built 18 Nov ([fx-modulation.md](fx-modulation.md#ensemble)): S1 to S4 and every FX row met; a mode of the Chorus family |
| 5 | Strings engine | Bank, registers, swell, bow noise, the Ensemble; 24 presets. Built 18 Nov ([synths.md](synths.md#strings-mpfree-synth-strings)): S6 to S9 met, S5 not (its cost grows with the notes heard; see [Synth quality](synth-quality.md#wave-1-targets-bass-keys-and-strings-stage-4)) |
| 6 | Bass engine | Synth and 808 bodies, Reese; 32 presets. Built 18 Nov ([synths.md](synths.md#bass-mpfree-synth-bass)): B1, B3, B4, B7, B10, V7 and L11 met; B5 on 21 of 32 presets (not the plain subs and clean 808s, by design; see [Synth quality](synth-quality.md#wave-1-targets-bass-keys-and-strings-stage-4)) |
| 7 | Keys | Velocity range and tone, sustain, the clav; presets to 32. Built 18 Nov ([synths.md](synths.md#keys-mpfree-synth-keys)): the clav (a waveguide string, two pickups, the tone switch, Mute, Release drop), Velocity per preset, each model voiced evenly across the keys, a Clav page; K1, K3, K4, K6, K7 and K9 met on every preset they apply to. The sustain pedal is the host's (Keys leaves it to `Node::hears`'s fallback) |
| 8 | Sounds | The Sound wrapper, macros and effects in presets, the macros page; factory presets as files. In two pull requests. The first is built (18 Nov, [Presets](presets.md#sounds-an-engine-with-its-own-effects-and-macros)): the wrapper, the file lines, the macros page first in the Synth tab, a bench case (the wrapper adds about 1 % to Keys with its Ensemble), no allocation while sounding, and the app's pads, tracks' copies, projects and auto-sampler holding sounds. The second moves the factory presets to files, with the first factory sounds that bring their own effects and macros |
| 9 | The preset browser (stage 6) | Try, Load and Save wired, the automatic preset checks (with the browser's owner) |
| 10 | Wave 1 listening and device asks | Blind A/B pages; the device round's cost asks |

**Wave 2: Poly with tables, FM, organ, wind and choir** (next week)

Poly and Wavetable merged with classic waves from tables (the aliasing fix), the supersaw, a second LFO and routes; FM's macros, shapes, DC and level fixes and less feedback aliasing; Drawbar's combo model; the Wind and Choir engines; Character; their presets.

**Wave 3: drums, plucks and leads**

Drums' own knobs, effects per drum, accent and variation, the shared 808 body; Pluck's fixes; expressive lead presets.

**The CPU work, alongside the waves**

| # | Pull request | When |
|---|---|---|
| C1 | Synth tracks on the sound cores (groups, placement by cost, per-core counts), off by default behind the Sound cores modes, with a `--lanes` bench of eight synth tracks | Wave 1; on by default only after the device round and Jay's pick |
| C2 | Flush-to-zero on the audio threads, the note-start bench, spreading start work | Wave 1 |
| C3 | `mpfree-simd`: one kernel (the supersaw's saws) and its twin, checked under qemu-arm; kept only if it wins | Wave 2 |
| C4 | A heavy synth split across two cores | If the device asks for it |
| C5 | Rendering ahead | If the device asks for it |

**What waits on the device:** voice defaults per engine, the per-core lines, whether synths go on core 1, and whether NEON pays. The asks for the next device round are in the project files; nothing new runs on the MPC.

## Sources

- The research pages: [Bass](../research/bass.md), [Keys](../research/keys.md), [String machines](../research/string-machine.md), [Poly and wavetable](../research/poly-synth.md), [FM](../research/fm.md), [Organs, winds, choirs and tape](../research/organ-wind-choir.md), [Drums](../research/drums.md), [What every type shares](../research/synth-topics.md), with their sources
- The baseline: [Synth quality](synth-quality.md); the bench's voice costs (`mpfree-bench --afford`)
- MPFree design pages: [Synths](synths.md), [Presets](presets.md), [Track host](track-host.md), [Concurrency](concurrency.md), [Profiling](profiling.md), [Modulation](modulation.md), [Wavetable](wavetable.md), [Layout spec](layout-spec.md)
- J. Parker, V. Zavalishin, E. Le Bivic, "Reducing the aliasing of nonlinear waveshaping using continuous-time convolution", DAFx 2016: https://dafx.de/paper-archive/details/vem_XXF5qBbfiWOH2RVVAA
- LLVM, "[ARM] Adding IEEE-754 SIMD detection to loop vectorizer" (r266363, 2016): https://lists.llvm.org/pipermail/llvm-commits/Week-of-Mon-20160411/347656.html
