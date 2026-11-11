# Sampler architecture

**Status:** proposal, 2026-11-10, for the owner's review. This is the engineering plan behind the [sampler design](sampler-design.md) and the [feature map](sampler-features.md). Mockups of the screens are in the [mpfree-claude repo](https://github.com/jferments/mpfree-claude/tree/main/sampler) together with the sampler skill tree.

It says which crates hold what, which thread does what, how data moves between them without ever blocking the audio thread, what is stored on disk, how each algorithm works in outline, and how each part is tested before it reaches the MPC.

## Principles

1. **The audio thread only mixes.** It reads capture, renders voices, writes playback and copies into fixed buffers. It never allocates, frees, locks, logs, touches files or waits on another thread. This is already MPFree's rule; the sampler keeps it with tests (below).
2. **Everything heavy is a background job.** Loading, decoding, analysing, stretching, rendering and saving run on worker threads and publish results when done. The sound you have keeps playing.
3. **State lives in one place.** The project model (sources, regions, pads, chop sets) is owned by the *control* thread. The audio thread gets immutable snapshots of what it needs; the UI gets change notices. Nobody else mutates the model.
4. **Every change is a command.** UI, hardware and MIDI all send the same commands to the control thread. Commands are logged for undo and are the only way to change a project.
5. **Testable on the workstation.** Every crate except the device backends runs and is tested on x86 and as 32-bit ARM under qemu, as today.

## Crates

```text
                 ┌──────────────────────── mpfree-app (the program on the MPC) ───────────────────────┐
                 │  wires threads together; owns start-up, shutdown, the supervisor heartbeat         │
                 └────────────────────────────────────────────────────────────────────────────────────┘
   UI side                         control side                             audio side
┌────────────────┐   ┌───────────────────────────────────┐   ┌───────────────────────────────────────┐
│ mpfree-ui       │   │ mpfree-sampler::model              │   │ mpfree-sampler::engine                 │
│  widgets, pages │   │  Project, Source, Region, ChopSet, │   │  VoicePool, Voice, modes, envelopes,   │
│ mpfree-sampler- │◄─►│  Layer, Pad, Program, commands,    │──►│  filters, the mixer, rolling buffers   │
│  ui (screens)   │   │  undo log                          │   │ mpfree-grain                           │
└────────────────┘   │ mpfree-project (files, TOML, save) │   │  grain scheduler and grain voices      │
                      │ mpfree-analysis (peaks, onsets,    │   │ mpfree-audio                           │
                      │  tempo, pitch)                     │   │  AudioDevice trait: ALSA playback +    │
                      │ mpfree-stretch (Signalsmith glue)  │   │  capture (device), file / null (tests) │
                      └───────────────────────────────────┘   └───────────────────────────────────────┘
   input: mpfree-controller (pads, Q-Links, wheel, buttons, LEDs) · mpfree-touch (touch)
```

| Crate | New? | Holds | Depends on | Unsafe |
|---|---|---|---|---|
| `mpfree-sampler` | grows from today's prototype | `model` (project types, commands, undo) and `engine` (voices, mixer, rolling buffers) | `mpfree-grain`, `basedrop`, `rtrb` | none (`forbid`) |
| `mpfree-grain` | new | Grain scheduler, windows, grain voice | none | none |
| `mpfree-analysis` | new | Peak pyramid, onset detection, tempo, pitch (YIN), zero crossings, loudness | `realfft` | none |
| `mpfree-project` | new | Project folder, `project.toml`, atomic saves, source files (WAV/FLAC) | `hound`, `claxon`, `flacenc` (optional), `toml`, `serde` | none |
| `mpfree-stretch` | new, later (S7) | High-quality offline time stretch and pitch shift | `signalsmith-stretch` (C++ inside) | in the dependency only |
| `mpfree-audio` | new, split from `mpfree-hello/src/alsa.rs` | `AudioDevice` trait (ADR-0002); ALSA playback **and capture**; a file device and a null device for tests | `libc`-level ioctls via `sys` | yes, isolated in `sys.rs` as today |
| `mpfree-sampler-ui` | started | The sampler's screens. Built: the pad editor's Pads tab (`pads_tab.rs`), its Wave tab (`wave_tab.rs`), its Chop tab (`chop_tab.rs`), its Play tab (`play_tab.rs`, with its pictures in `play_widgets.rs`) and `SourceView` (peaks, mono mix, onsets, tempo and root note per source, worked out off the audio thread). Still to come: Grain and FX tabs, Record, Browser, Perform | `mpfree-ui`, `mpfree-sampler`, `mpfree-analysis` | none |
| `mpfree-ui` | exists | New widgets: `Waveform` (with the marker ruler as its tab strip) and `WaveOverview` are built in `wave.rs`, drawn from a `Peaks` source with a playhead redrawing only its strips (`examples/wave_preview.rs` draws the Wave tab to a PNG); still to come: `Meter`, `XYPad`, `EnvelopeEditor`, `LayerMap`, `QLinkStrip`, `PadGrid` additions (waveform thumbnail, pressure ring) | `fontdue` | none |
| `mpfree-app` | new, grows out of `mpfree-hello` | The program: threads, wiring, settings, start-up | everything above | none |

`mpfree-hello` stays as the test program for hardware rounds; `mpfree-app` is what the owner eventually boots into.

## Threads

| Thread | Priority | Core | Period | Owns |
|---|---|---|---|---|
| Audio | `SCHED_FIFO` 70, memory locked | 3 | every ALSA period (128 frames, 2.9 ms) | The `Engine`: voice pool, mixer, rolling buffers. Reads capture, writes playback |
| Control | normal, high nice | 1 | event-driven, at most every 1 ms | The `Project` model, the command queue, the undo log; turns hardware and UI events into commands |
| Workers (2) | normal, low nice | 0 and 1 | job queue | Loading, decoding, analysis, stretching, rendering, saving, recording to disk |
| UI | normal | 0 | redraw at most every 33 ms, only what changed | The `Ui` from `mpfree-ui` and the sampler screens |
| Input | normal | 1 | blocking reads | Controller MIDI and touch events, forwarded to Control (pad hits also go straight to Audio, see below) |

**Pad latency path.** A pad hit must not wait for the control thread. The input thread sends the hit to the audio thread directly on a dedicated ring (`PadHit { pad, velocity, time }`), and a copy to Control for the UI and recording. The audio thread looks the pad up in its own snapshot. This keeps the pad-to-sound path as short as the pad player's today; round 4 measures that path on the device.

## Data flow

```text
                 commands (rtrb, bounded)                     snapshots (rtrb of Arc-like handles)
   UI ──────────────► Control ───────────────────────────────────────────────────► Audio
   Input ───────────► Control         ▲                                               │
   Input ── PadHit ───────────────────┼───────────────────────────────────────────►  │
                                      │ jobs                                          │ events (rtrb, try_send, drop + count when full)
                                      ▼                                               ▼
                                   Workers ──── results ────► Control ────► UI    Control (meters, playheads, takes)
                                      ▲
   Audio ── capture ring (rtrb, 2 s) ─┘  (recording to disk)
```

Every queue is a bounded single-producer single-consumer ring (`rtrb`). The audio thread only ever uses non-blocking `push`/`pop`; when a ring to the UI is full it drops the message and counts it, as the pad player does.

### Snapshots, not shared mutation

The audio thread holds an `EngineView`: for each pad, the layers and regions it needs, already resolved into plain numbers and handles to sample memory. When the model changes, Control builds a new view for the changed pads off the audio thread and sends it. The audio thread swaps it in at the start of the next period. The old view is returned through a ring to Control, which drops it there, so the audio thread never frees memory. (`basedrop`'s collector does the same job for shared sample buffers.)

A playing voice keeps its own handle to its sample buffer, so replacing or deleting a sample while it plays is safe: the buffer is freed only when the last voice stops.

### Sample memory

| Item | Plan |
|---|---|
| Format in RAM | Interleaved stereo `f32`, 44.1 kHz. A mono source is stored mono and read with a channel flag |
| Pool | One `mlock`ed arena reserved at start-up (size from settings, default 1 GiB, measured on the device), carved into buffers by a simple allocator on the control thread. No allocation from the system heap per sample, so memory use is predictable and never pages |
| Handles | `SampleRef` = index + generation into the arena, reference-counted; the audio thread only increments and decrements counts, and frees go back through a ring |
| Long sources | First version: everything in RAM. Later (S8): sources over a size limit stream from disk with the first second resident |

On a 32-bit process the address space is about 3 GB. A 1 GiB arena, the rolling buffers (about 85 MB), the UI (under 64 MB) and the engine (under 64 MB) fit with room to spare.

## The model

Sketches of the types, to fix the vocabulary. Names may change in code.

```rust
struct Source  { id: SourceId /* content hash */, frames: u64, channels: u8, rate: u32,
                 file: PathBuf, analysis: Option<AnalysisRef>, name: String, tags: Vec<String> }

struct ChopSet { id: ChopId, source: SourceId, name: String, mode: ChopMode, markers: Vec<u64> }

struct Region  { source: SourceId, start: u64, end: u64,
                 loop_: Option<Loop /* start, end, crossfade, mode: Forward | PingPong */>,
                 fade_in: Fade, fade_out: Fade, gain_db: f32, reverse: bool,
                 channels: ChannelPick, tune_cents: i32, mode: PlayMode, warp: Option<Warp>,
                 grain: Option<GrainParams>, colour: Option<Vintage> }

enum PlayMode  { OneShot, Gate, Toggle, Loop, Tape, Granular }

struct Layer   { region: RegionRef, vel: (u8, u8), xfade: u8, group: LayerGroup /* Velocity | Cycle | Random | Simult */,
                 level_db: f32, pan: f32, tune_cents: i32 }

struct Pad     { layers: Vec<Layer>, poly: Poly, choke: Option<u8>, choke_one_way: bool, links: Vec<PadId>,
                 amp: Ahdsr, filter: Filter, filter_env: Ahdsr, pitch_env: Option<Env>,
                 vel_to: VelocityMap, pressure_to: Option<Target>, snapshot: Option<Box<Pad>> }

struct Program { kind: ProgramKind /* Drum | Keygroup */, pads: [Option<Pad>; 128], name: String }
```

**Commands** are an enum (`SetRegionStart { region, frame }`, `AddMarker`, `Convert { chop, to }`, `AssignToPad`, `RecordStart`, ...). Each command applied to the model returns its inverse, which goes in the undo log. Live gestures (a knob turning) are merged into one undo step per gesture.

## Recording

```text
 audio thread, every period:
   capture[128 frames] ──► input rolling buffer (ring, 2 min)        (always)
   master[128 frames]  ──► master rolling buffer (ring, 2 min)       (always)
   if a take is armed:  ──► take ring (rtrb, 2 s) ──► writer worker ──► arena buffer (+ WAV on disk for long takes)
   threshold armed:     detect crossing on capture (peak of block > level) → mark take start = onset − pre-roll
```

- **Rolling buffers** are plain ring buffers of `f32` in the locked arena, written with a wrapping index. A "skip back N seconds" or "last N bars" command from Control copies the span out on a worker while the ring keeps going; the ring is long enough (2 minutes) that the span cannot be overwritten during the copy for any span up to 1 minute.
- **Threshold** compares the block peak with the level on the audio thread (cheap), and the exact onset inside the block is refined later by the analysis.
- **In-time takes** start and stop on bar lines computed from the transport's sample-frame position, so the take is an exact number of bars.
- **Capture device.** New in `mpfree-audio`: open `pcmC1D0c` (the ACVA capture PCM, observed stereo S32_LE at 44.1 kHz, 128-frame periods), link it to playback so both start together, and read one period per playback period. Capture and playback xruns are counted separately.
- **Writes to disk** only on the stick or SD card, never on the eMMC. On the 3.10 test build all paths are in RAM. The writer is the only code that opens a file for writing in the recording path, and the static test that lists write sites covers it.

## Playback engine

### A period, in order

1. Swap in any new `EngineView`, apply parameter changes and voice commands from the rings.
2. Handle pad hits (from the dedicated ring and the sequencer): choose layers, steal or choke voices, start voices.
3. Render each active voice into its pad's bus (block-wise, 128 frames, or split at sample-accurate event times).
4. Pad buses through pad FX, then into the master; master into playback.
5. Copy capture and master into the rolling buffers; push take frames.
6. Push meters, playheads and grain positions to the UI ring (at most once per UI frame's worth of periods).

### The voice

- **Position:** 32.32 fixed point (integer frame and fraction), increment from pitch × rate ratio. Fixed point avoids `f64` drift on long loops and is cheap on ARMv7.
- **Interpolation:** linear (cheap), 4-point Hermite (default), with sinc only in offline rendering. Reads at the edges use guard frames so there is no branch per sample.
- **Loop:** forward or ping-pong between loop start and end, with an equal-power crossfade of N frames precomputed as a short table at region change.
- **Envelopes:** AHDSR as a per-sample multiply with a per-block recomputed segment rate (exponential segments as one multiply per sample).
- **Filter:** a topology-preserving state-variable filter (LP/HP/BP, 2-pole; 4-pole as two in series), coefficients updated per 16-frame sub-block when modulated.
- **Declick:** 64-frame release on stop and 32-frame fade on steal or choke, as in today's `mpfree-sampler`.
- **Pressure:** each pad has one pressure route (`Pad.pressure`: a `PressureTarget` and a depth from −1 to 1). `Event::Pressure { pad, value }` (0-127, from the pad's polyphonic aftertouch) moves that pad's playing voices; a voice that starts later begins unpressed. Pressure is smoothed with a 5 ms one-pole. Full pressure at depth 1 gives +12 dB level (the gain glides over each 16-frame sub-block, so it never clicks), +12 semitones pitch (sample voices change their step; granular voices change the grain pitch), +4 octaves of filter cutoff (needs the pad's filter on), ×16 grain density, ×8 grain length, the grain position moved by the whole region (wrapping), or +100 % spray. A negative depth moves the other way. Built and tested (`tests/pressure.rs`); it also runs in `tests/no_alloc.rs`.
- **Voice allocation:** a fixed pool (start at 64, set by measurement). Stealing order: same pad's oldest voice if the pad's polyphony is exceeded, then the quietest releasing voice, then the oldest.

### In the pad player

From round 6 the pad player (`mpfree-hello`, `src/sampler.rs`) plays through this engine instead of its round 3 mixer. The `drums` step keeps the old mixer.
- Each kit in `kits.bin` becomes a project with one engine view, built before the sound starts. Picking a kit swaps views, so nothing is built, allocated or freed in the audio loop, and sounds already playing finish.
- Levels match the old mixer: pads are raised 3.01 dB to undo the centre pan, and velocity is sent as `round(127·√(v/127))`, so the engine's squared level law comes out linear.
- A Grains kit plays the first kit's sounds as granular pads while they are held. Pressure moves density, grain size, position or pitch, one per row of pads.
- Pad pressure from the controller goes to the engine as `Event::Pressure`, and Note Repeat's note-offs go to it as `Event::NoteOff`.
- **Pads on the mixer.** `Engine::render_routed(frames, out)` renders each pad apart: every voice adds its sound into its own pad's pair from a `PadOut` (the mixer's pad channels, `mpfree_fx::Mixer::pad_input`), so each pad gets the mixer's fader, pan, mute, solo, sends, inserts and output. Events, voices and grains run as in `Engine::render`, and the pads' pairs add up to what it renders (`tests/routed.rs`). The engine never clears a pair (the mixer clears its own after each block). A pad with no pair (`None`) still plays, into a scratch buffer, so its voices end on time. The master grain effect works on the mixed pair, so it runs only in `Engine::render`; on the mixer a granular insert on the pads' track does its job. `sampler::Player::render_routed` passes it on for the playing kit's 16 pads; the FX thread's wiring puts them on pad channels 0 to 15, each going to the Drums track. `render_dry` stays for tests and previews. Routed renders run in `tests/no_alloc.rs` too.
- The 30 s status line adds the engine's render time per period (mean and max, and as a share of the 2.9 ms period), steals, dropped events, and peak and skipped grains. On the RK3288, these figures set the voice and grain limits.

### The Wave tab (`mpfree-sampler-ui`)

The first of the pad editor's tabs, as in the Wave mockup. It shows one layer of one pad: the overview, the waveform with Start, End and loop markers, the fades, bar and beat lines from the source's tempo, the Region settings and four Q-Link cards.
- The tab never owns the project. The app builds a `WaveState` from the project and a `SourceView`, and applies the `WaveAction`s it gets back with `WaveAction::apply`.
- Each drag or Q-Link turn is one undo step: the tab gives every change of the same drag or turn the same gesture id, and the app calls `lift` when a finger lifts or a Q-Link is let go.
- Markers can't cross: Start ≤ loop start − crossfade, loop start < loop end, loop end ≤ End. Fades shrink to fit. Every move gives a region that `Region::check` accepts (the test makes 2000 random moves).
- Markers snap to the nearest zero crossing (the default), to sixteenths of the source's tempo, or not at all. A Q-Link moves its marker one pixel of the current zoom per detent, snapping in the direction of the turn. With SHIFT it moves one frame per detent, unsnapped. The wheel zooms round the marker last moved or touched.
- Region rows: mode, loop (off, forward, ping-pong), crossfade, gain (0.5 dB steps), tune (semitones), reverse, channels, fade in, fade out and snap. Loop mode always keeps a loop: choosing it makes one over the middle half of the region, and Off is skipped while it is on.
- A tap on the wave asks the app to play the pad. Playheads come from the app with `set_playheads`.
- `mpfree-sampler-preview OUT_DIR` draws the sample screens to PNG files.

### The Chop tab (`mpfree-sampler-ui`)

The pad editor's second tab: it cuts what the pad plays (its region, Start to End) into slices and puts them on pads.
- **Methods.** Transient (the source's onsets, at a sensitivity from 0 to 100 %), Beat (on the source's own beat grid, the bar lines the Wave tab draws, every 16th, 8th, beat, 2 beats or bar; offered only when the source has a tempo), Grid (2 to 64 equal slices), Silence (one slice per sound, at a threshold from −72 to −12 dB) and Manual. Cuts are at least 10 ms apart and from the region's ends; at most 64 slices.
- **Hand edits.** Dragging a slice's marker, Q-Link 2, Split (the slice picked, near its middle) and Merge (with the slice before) change the cuts and turn the method to Manual, so changing the method's setting never silently undoes them. Beat, Grid, Split and hand moves snap to zero crossings unless Snap is off; transient onsets already sit just before each attack. The first marker is the pad's Start and END its End: they stay put (Start and End move on the Wave tab).
- **The cuts live in the tab** until Chop to pads: trying methods changes nothing in the project. Another pad, or a moved Start or End, chops again (a Manual chop keeps the cuts still inside).
- **Chop to pads** (`ChopAction::ToPads`, applied with `ChopAction::apply`) adds a chop set (markers at Start, the cuts and End) and puts its slices on consecutive pads of the chopped pad's bank of 16, as one undo step (`Project::chop_to_pads`). Each new pad is a copy of the chopped pad (level, envelopes, filter, choke, and its sound's gain, tune and channels) with one layer playing its slice once, named after the pad and numbered from 1. The first pad is the one after the chopped pad if the slices fit after it, else the last run that fits; Q-Link 4 and the Pads row move it. The list says which pads already have a sound ("replaces A5"); more than 16 slices can't go on pads.
- **Q-Links** pick a slice, move its start, turn the method's setting and move the first pad. The wheel zooms round the slice picked; a tap on the wave picks the slice under it.

### The Play tab (`mpfree-sampler-ui`)

The pad editor's third tab: how the pad plays its sound. It edits the whole pad, never a layer's region (that is the Wave tab's), and `PlayAction::apply` keeps the project's regions whatever the action carries.
- **Layers.** The layer map draws each layer on a velocity axis (0 to 127), one row each, with a line at the last hit's velocity. Dragging a layer's left or right edge sets the lowest or highest velocity it plays (layers may overlap; the high edge can't cross the low one). A tap picks the layer the envelope is drawn over. The Layers row says how matching layers play: Stacked (all), Cycle (round robin) or Random.
- **Amp envelope.** Drawn over the picked layer's sound from its Start: attack is linear, decay and release fall exponentially to about −80 dB in their time, as the engine's envelope does (`env_level`). Sustain sits where the sound ends (or after the decay, if that is later) and Release follows it. Dragging A, H, D or R moves that time, S its level; the time axis holds still while a point is dragged. Times go up to 20 s; release is at least 1 ms.
- **Filter.** Its response on a log axis from 20 Hz to 20 kHz, as the pad's state-variable filter shapes it (`filter_db`; 4 poles are two 2-pole stages). Dragging the dot moves the cutoff. Turning the filter on while its cutoff is still at 20 kHz starts it at 2 kHz (a high-pass at 200 Hz), so the change is heard.
- **The Pad list.** Polyphony (mono to 32 voices), Mute group (off, 1 to 16), Layers, Level (−48 to +12 dB), Pan, Tune (whole semitones, ±48), Vel → level, Vel → start, Pressure (Level, Pitch, Filter cutoff, and the grain targets for a granular pad) and its Depth, Attack, Hold, Decay, Sustain, Release, Filter (LP, HP or BP, 2 or 4 poles), Cutoff (a third of an octave a step), Resonance and Filter env (±4 octaves). A tap picks a row and its arrows step it; the wheel steps the row picked.
- **Q-Links** turn Attack and Decay (a doubling every 8 detents, at least a millisecond each; SHIFT: 1 ms a detent), Cutoff (a semitone a detent; SHIFT: a tenth) and Resonance (1 %; SHIFT: 0.2 %). Cutoff and Resonance wait while the filter is off. A tap on a Q-Link's card picks its row.
- **Undo.** Each drag, Q-Link turn, wheel turn or run of taps on one row is one undo step; the app ends it when the finger lifts or the control rests.

### The Pads tab (`mpfree-sampler-ui`)

The pad editor's first tab, a sub-page of the Sample tab (Jay, 2026-11-11: a graphical pad editor that doesn't take up the whole screen). The pad bar reads Pads, Wave, Chop, Play, Grain, FX; the Hello MPC thread, which owns the layout, puts the order to Jay. The page is three columns: the pad map (440 px), the Pad list and the kit's sounds (340 px).
- **Pad map.** The bank's 16 pads in a 4×4 grid laid out as on the MPC (pad 1 bottom left), each with its name, its layers and play mode ("3 layers · Loop"), and the clip it launches. A tap picks a pad for every tab (Wave, Chop and Play edit it next); a tap on the pad already picked plays it while the finger is down. Pads held on the hardware light up, and hitting a pad picks it as on the other tabs.
- **Clips.** The time engine (`mpfree-seq`, `mpfree-play`) owns clips and doesn't launch them from pads yet, so the Pad list says "clip: none" and the map shows no clips. `PadsState::clips` is where the app puts each pad's clip name once the time engine has an accessor for it.
- **Copy, Paste, Clear** work on the pad picked, each one undo step. Paste (named after the pad copied, such as "Paste A1") puts the copy, with its sound, layers and every setting, on the pad picked; Clear leaves it without a sound.
- **The Pad list.** Sound, Mode (One shot, Gate, Loop, Granular; Loop adds a loop where a layer has none), Polyphony, Mute group, Level, Pan and Tune. Its rows step as on the Play tab (`play_tab::stepped`). Stepping Sound puts the kit's next or previous sound on the pad.
- **Sounds.** Every sound in the kit's project, with its length and the pads that use it. **Put on** puts the sound picked on the pad picked (`with_sound`): the first layer plays it from start to end with no fades, the pad keeps its settings and the old sound's gain, tune, channels and play mode, and takes the sound's name. On an empty pad it makes a new one-shot pad.
- **Q-Links 1 to 4** turn Level, Pan, Tune and Polyphony; the **wheel** steps the row picked. A run of turns or taps on one row is one undo step, as on the other tabs.
- **The other tabs** are greyed out while the pad picked has no sound.

### The Sample tab in the pad player (`mpfree-hello`)

The pad player's top bar has a Sample tab (Jay's name for the pad editor). It sits after Synth for now; the approved layout spec puts it after Looper, and the Hello MPC thread, which owns the bar, moves it there. It shows the pad editor (Pads, Wave, Chop or Play, from the pad bar's tabs) for the playing kit's pad that was hit last, or picked on the Pads tab.
- **Who owns what.** `sampler::Player::new` builds one project per kit (and the Grains kit) and hands them to the screen thread with `take_projects`. The screen thread owns them from then on (`sample_page.rs`); their undo history starts there, so UNDO never takes a loaded kit apart (`Project::clear_history`). The audio is shared with the engine's views (`Arc`), not copied.
- **Edits reach the sound.** An edit changes the project on the screen thread, which then builds the kit's `EngineView` again and sends it to the audio thread in a box (`FromUi::SampleView`). `Player::swap_view` puts it in place (in the engine, if the kit is playing) and leaves the old view in the same box, which goes back (`ToUi::SampleViewBack`) to be freed on the screen thread. One view travels at a time; edits made meanwhile go with the next one. The audio thread never builds, allocates or frees a view; if the screen's channel is full, it keeps the box and tries again next period. Sounds already playing finish as they started; the next hit plays the edit.
- **Controls.** On Wave, Q-Links 1-4 move Start, End, loop start and loop end (SHIFT: one frame a detent); on Chop they pick a slice, move its start, turn the method's setting and move the first pad; on Play they turn Attack, Decay, Cutoff and Resonance; on Pads they turn Level, Pan, Tune and Polyphony. Letting go of a Q-Link, or a second without turning, ends the turn (one undo step). The wheel zooms on Wave and Chop and changes the row picked on Play (a turn is one undo step, like a Q-Link's). SHIFT + wheel and - / + pick the previous or next pad with a sound. UNDO and SHIFT + UNDO undo and redo on this tab (the time engine's takes elsewhere). A tap on the wave plays the pad; in Gate, Loop and Granular modes it keeps sounding until the next tap or leaving the tab. Opening the tab while the synth has the pads gives them back to the last drum kit.
- **On screen.** Pad names use the kit's bank letter (kit 2's pad 5 is B5). The playing kit's playheads come from the audio thread about 25 times a second while the tab shows (`Playheads`, fixed size). A pad without a sound opens on the Pads tab, to put a sound on it; while the synth has the pads, a page says so.
- **In the log.** Each finished drag or turn writes one `sample:` line with what changed (for example `sample: kit 1 (House) pad A1 Kick: Start 0.000 s -> 0.012 s`), and the session's last lines count the views swapped in.
- **Chop.** Chop to pads changes the kit's project like any edit, so its new view goes to the audio thread the same way. `Player::swap_view` takes which pads have a sound from the view (`EngineView::has_pad`), so the new pads play, and the playing kit's pad lights follow (lit in the kit's colour where there is a sound; the same after an undo). The Play page's pad names follow too (`SamplePage::take_renamed`). The log gets one line per chop, such as `sample: kit 1 (Breaks) pad A5 Break 92: chopped 0.163 s to 4.891 s into 4 slices (Grid) on pads A6 to A9`.
- **Play.** A hit's velocity shows on the layer map. Each finished drag or turn writes one line with what changed, such as `sample: kit 1 (Breaks) pad A5 Break 92: decay 0 ms -> 8 ms`.
- **Not yet.** Edits live in memory until the power goes off (no saving yet). A tap on the Chop tab's wave picks a slice but doesn't play it yet. Grain and FX are greyed out. The Play tab doesn't add or remove layers, set a layer's own level, pan or tune, the filter envelope's times or the one-way chokes yet.
- **Pads.** A tap on a pad picks it for every tab; a tap on the pad picked plays it as a hit would (it lights, and lets go when the finger lifts). Copy, Paste, Clear, Put on and each run of turns write one line, such as `sample: kit 1 (Breaks) pad A3: now plays Hat closed` or `sample: kit 1 (Breaks) pad A1: cleared (was Kick 58)`. The Play page's pad names and lights follow, as after a chop.
- **Not yet on Pads.** The clip each pad launches (waits on the time engine), dragging one pad onto another, the Q-Link displays beside the knobs (Jay asked for them; the Hello MPC thread is drawing layouts, and all the sampler's pages will move together).

### Warp and stretch

- **High quality:** `mpfree-stretch` runs Signalsmith Stretch on a worker to render a stretched copy at the project tempo. When the tempo changes, the region keeps playing with the real-time stretch below until the new copy is ready, then crossfades to it at the next loop start.
- **Real time:** the granular engine in "stretch" mode (grain size 40-90 ms, overlap 4, scan speed = tempo ratio, pitch 0, position jitter 0), which is a form of synchronous granular stretching (close to WSOLA without the cross-correlation search). Cheaper and lower quality; good for live tempo moves and as a deliberate effect.

## Granular engine (`mpfree-grain`)

```text
 GrainScheduler (per granular voice)
   every block: accumulate density × block time; for each due grain:
     pos   = playhead + jitter(spray)            (playhead advances by scan_speed)
     pitch = base + pressure/knob mod + random(pitch_spray), quantised to scale if set
     len   = size (frames), dir = reverse? with probability, pan = random(spread)
     push Grain { pos, inc, remaining, window_phase, window_inc, gain_l, gain_r } into a fixed array
 Grain render: for each active grain, for each frame:
     s = hermite(buffer, pos); w = window_table[phase]; out_l += s·w·gain_l; out_r += s·w·gain_r
```

- **Fixed arrays:** each granular voice has up to 64 grains; there is a global grain budget across voices (start at 256, set by measurement). Past the budget, new grains are skipped, never queued, so the period is never at risk.
- **Windows** are 1024-entry tables (Hann, Tukey with a shape parameter, trapezoid, exponential), read with linear interpolation.
- **Sources:** a region (granular playback mode) or a circular live buffer of 4-8 s (granular effect). Freeze stops writes to the live buffer.
- **Sync:** density and size can lock to tempo divisions, using the transport's frame position.
- **Determinism:** randomness comes from a seeded generator per voice, so offline renders are reproducible in tests.

**Built (S6, workstation half):** `rust/mpfree-grain`, with the tool `mpfree-grain` in `mpfree-sampler` for listening to eight presets on any WAV. What it does, as built:

- `GrainVoice` has a fixed array of 64 grains. `Budget` counts the grains alive across all voices and refuses new ones past its limit.
- Grain sources are `Clip` (a region of a sample) and `LiveBuffer` (a circular buffer with freeze). Grains are placed so the whole read stays inside the readable span, so a grain is never cut short or reads outside its region.
- Windows are Hann, Tukey, trapezoid and exponential, as 1025-entry tables. Pitch can be quantised to a 12-note mask. Density and size can lock to the transport's frame timeline (`TempoSync`), and synced grains start on exact frames whatever the period size.
- Pads play it through `PlayMode::Granular` in `mpfree-sampler`. Each voice slot has its own `GrainVoice`, and one `Budget` (`GRAIN_BUDGET`, 256) is shared by all of them. A region keeps its grain settings in every play mode. Tune adds to the grain pitch, reverse makes every grain play backwards, and the pad's envelope, filter, pan, chokes and polyphony apply as for any other voice. Lifting the pad stops new grains and releases.
- **The granular effect** (`GrainFx`) is an insert on a stereo signal. It keeps the last 0.5 to 30 s in a `LiveBuffer`, plays a `GrainVoice` from it, and mixes the grains back with an equal-power mix that glides over 10 ms. Freeze stops the buffer taking new audio, so the cloud keeps playing what it holds while the dry signal carries on. Audio reaches the buffer in fixed 64-frame chunks counted from the effect's first frame, so the output doesn't depend on the period size; new audio waits at most 1.5 ms before it can be granulated, and freeze takes in what is waiting first.
- **For now it sits on the sampler engine's master.** `MasterFx` in the project holds the mix and grain settings (undoable, with a knob turn merged into one step), and the engine view carries them. `Engine::make_master_fx` makes the effect with 8 s of memory (2.8 MB, off the audio thread), `Engine::set_master_fx` puts it in (it glides in from dry) or takes it out, and `Event::Freeze { on }` freezes it on an exact frame. It runs after the voices on every segment between events, and its grains come out of the engine's grain budget. This is an interim hook: the FX chain (`mpfree-fx`, on the shared `mpfree-node` interface) will own the mixer and master bus, and `GrainFx` becomes one of its nodes there. Tests: `mpfree-grain/tests/fx.rs`, `mpfree-sampler/tests/master.rs` and both no-allocation tests; the listening render is `master-freeze.wav` from `mpfree-grain`.
- **As an FX-chain node.** `GrainNode` (`mpfree-grain/src/node.rs`) wraps a `GrainFx` with 8 s of memory and its own budget of 64 grains, as an `mpfree_node::Node` (id `mpfree.grains`), so it can sit on an insert, a send return or the master chain. Its parameters are in plain units: Mix, Position, Spray, Reverse and Spread in per cent, Size in ms, Density in Hz, Pitch and Pitch spray in semitones; Freeze is a toggle a sequencer can set on an exact frame, and Grains is a read-only meter of the grains playing. `prepare` allocates the memory; `process`, `set_param` and `reset` don't allocate. Parameter events land on their frame, the output is the same at any block size (to within 1e-4), and `reset` clears what it heard. Its tail is the memory plus one grain, endless while frozen and none when fully dry. Tests: `mpfree-grain/tests/node.rs` and `tests/no_alloc.rs`.
- Output doesn't depend on the period size, and the same seed gives bit-identical output. `tests/no_alloc.rs` covers render, start, stop, kill, parameter changes and live-buffer writes.
- Cost measured on the workstation (x86, one core, `cargo run --release -p mpfree-grain --example bench`): about 67 million grain-frames a second on a stereo clip and 59 million on a live buffer, which is 4-5 % of a core for 64 grains at 44.1 kHz. The RK3288 figure is still to be measured, and it sets the budget.

## Analysis (`mpfree-analysis`)

| Analysis | Method | Output |
|---|---|---|
| Peak pyramid | Per channel min, max, RMS as `i16` at block sizes 64, 256, 1024, 4096 frames | `PeakPyramid`, saved as `<source>.peaks` |
| Onsets | 1024-point FFT, hop 256 (5.8 ms); spectral flux on log-magnitude; adaptive threshold = median over ±8 hops × (1 + sensitivity); peak-picking with a 30 ms minimum gap; refine each onset to the nearest preceding low-energy frame | sorted frame positions with strength (so sensitivity can change without recomputing) |
| Tempo | Autocorrelation of the onset strength curve over 60-200 BPM, with a preference curve around 90-120 and octave checks; offered, not forced | BPM and confidence |
| Pitch | YIN on up to 24 windows of 100 ms within 24 dB of the loudest; voiced windows grouped by note, weighted by loudness × periodicity × steadiness (built: `mpfree_analysis::root_pitch`, also per slice via `SourceAnalysis::pitch_of`) | root note and cents, with confidence |
| Zero crossings | On demand within ±N frames of a marker | nearest crossing with matching slope |
| Loudness | Peak, RMS; integrated LUFS later | numbers for normalise |

Analysis results go in the source's cache file with a version number; a newer analyser re-runs old caches in the background.

## Files

```text
MyBeat.mpfree/
  project.toml          plain text, versioned (format = 1), written atomically (temp file, fsync, rename)
  undo.log              append-only command log, one record per line, fsync on each gesture end
  samples/
    3f9a…c2.wav         sources, named by content hash; WAV 24-bit or float, or .flac
    3f9a…c2.peaks       analysis cache (rebuildable)
    3f9a…c2.toml        name, tags, original file name, analysis version
  takes/                recordings not yet assigned (kept until the project is cleaned)
```

`project.toml` holds programs, pads, layers, regions, chop sets and settings, referring to sources by hash. A sample can be exported with a small sidecar (`.mpfree.toml`) that carries its regions, chop sets and warp, so edits travel with the file (an owner complaint about the MPC).

## Real-time guarantees and how they are tested

| Guarantee | Test |
|---|---|
| No allocation or free in the audio path | `assert_no_alloc` around `Engine::render` in tests and in a debug build; a counting global allocator in CI fails the test on any allocation during render |
| No locks, files or logging in the audio path | Static test: the `engine` module may not use `std::sync::Mutex`, `std::fs`, `println!`, or `log` (grep-based, like the existing write-site test) |
| Bounded time | Worst-case render benchmarks (all voices, all grains, all filters modulated) on the workstation and on the MPC; the device result sets the voice and grain limits |
| Same output on x86 and ARM | Offline renders hashed and compared, as M6 did for the groove |
| Deterministic | Seeded randomness; renders of a fixed command script are bit-identical run to run |
| Undo is exact | Property test: random command sequences, then undo all, compare with the start |
| Saves survive power loss | Kill-during-save tests on the workstation; the old file is always intact |

## Performance budget

| Item | Budget (first guess, to be measured on the RK3288) |
|---|---|
| Audio period | 2.9 ms; target render under 50 % (1.45 ms) with the full voice and grain load |
| One Hermite voice with envelope and filter | about 1-2 % of the period |
| 64 voices | about 40-60 % of the period, so the default pool may be 48, measured |
| One grain | a fraction of a voice (no filter); 256 grains about one third of the period |
| UI frame | under 15 ms on core 0; waveform redraw only for the changed columns |
| Pad to sound | no worse than the pad player's measured path |

If the budget fails on one core, the engine splits pad buses across cores 2 and 3 with a join before the master, as the earlier research proposed.

**Measured on the MPC One (round 7, 2026-11-11).** The bench case "Sampler, 20 pads" (16 sample pads with two layers and a low-pass filter each, 4 dense granular pads, 24 voices, a hit every 4 blocks) took 1437 µs on average, 50 % of a period, against the sampler's 25 % budget; with the master grains, 72 %. Per voice that was far slower than on the workstation, and the reasons were ARMv7's: every sample voice frame called an out-of-line read eight times (a clamp and a channel `match` per tap) and converted a 64-bit integer to a float twice (a library call on ARMv7), every grain frame converted between `f64` and `i64` twice (two more library calls), and the live buffer behind the master grains divided a 64-bit number per frame. The voice now reads both channels straight from the data, once, away from the region's edges (`Voice::read2`; the clamped read stays for the edges), takes the fraction from the position's low 32 bits, retunes its filter only when the envelope or pressure has moved it (and the second stage only for 4 poles), and keeps its per-16-frame updates out of the per-frame loop; grains go through `i32` below 2^31 frames; the live buffer keeps its write head instead of dividing. The output is bit for bit what it was (an output hash over 3000 varied blocks, mixed, with master grains and routed, matches on x86 and on ARMv7 under qemu; `the_direct_read_matches_the_clamped_one` and `live_buffer_finds_every_held_frame_after_wrapping_and_clearing` keep it so). On the workstation the 16 sample pads now take 58 µs instead of about 140, and "Sampler, 20 pads" 115 µs instead of 174; under qemu on the ARMv7 build, which counts instructions more than it times the chip, the case drops 31 %, the master grains case 44 % and the dense grain effect 42 %. The bench now also runs the two halves apart: "Sampler, the 16 sample pads alone" and "Sampler, the 4 granular pads alone".

The worst blocks come from the grains: the four granular pads average 50 grains but reach the 256-grain budget, and a block with 256 grains costs about five times the average. The budget is the lever for the worst case and is to be set from the next device bench.

## Failure handling

- **xrun:** count it, show it in the header, keep going. Never stop the transport for an xrun.
- **Worker failure** (bad file, failed stretch): the job reports an error to the UI; the model is unchanged.
- **Arena full:** a recording or load is refused before it starts, with the space needed and the space free; the rolling buffers are never shrunk.
- **UI crash:** the engine keeps playing; the supervisor restarts the UI (later, when the UI is its own process).
- **Power loss:** the last saved project and the undo log replay to the last finished gesture.

## Device safety

Nothing in the sampler writes to the eMMC. Recording, saving and caches write only to the project folder, which on the 3.10 test build is in RAM and later lives on the USB stick or SD card. Every round on the MPC runs the storage hashes before and after, as now. The capture PCM is opened only on the test build, where Akai's app is not running.

## Build order

The steps S1-S8 in the [design](sampler-design.md#steps) map onto crates like this:

| Step | Crates touched |
|---|---|
| S1 model and voices | `mpfree-sampler` (`model`, `engine`) |
| S2 peaks and waveform widgets | `mpfree-analysis` (peaks), `mpfree-ui` |
| S3 chop and convert | `mpfree-analysis` (onsets, tempo), `mpfree-sampler` (commands) |
| S4 capture, recording, skip back, resample | `mpfree-audio`, `mpfree-sampler` (rolling buffers, takes), `mpfree-project` (writer) |
| S5 edit screens, undo, save | `mpfree-sampler-ui`, `mpfree-project` |
| S6 granular | `mpfree-grain`, `mpfree-sampler-ui` (Grain tab) |
| S7 warp, phrase, keygroups, looper, tape, auto-sample | `mpfree-stretch`, `mpfree-sampler` |
| S8 streaming and imports | `mpfree-project` (`symphonia` optional), `mpfree-sampler` (stream voices) |
