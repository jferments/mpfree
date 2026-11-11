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
| `mpfree-sampler-ui` | new | The sampler's screens: Sound page tabs, Record, Browser, Perform | `mpfree-ui` | none |
| `mpfree-ui` | exists | New widgets: `Waveform`, `WaveOverview`, `MarkerRuler`, `Meter`, `XYPad`, `EnvelopeEditor`, `LayerMap`, `QLinkStrip`, `PadGrid` additions (waveform thumbnail, pressure ring) | `fontdue` | none |
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
- The 30 s status line adds the engine's render time per period (mean and max, and as a share of the 2.9 ms period), steals, dropped events, and peak and skipped grains. On the RK3288, these figures set the voice and grain limits.

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

**Built (S6, workstation half):** `rust/mpfree-grain`, with the tool `mpfree-grain` in `mpfree-sampler` for listening to six presets on any WAV. What it does, as built:

- `GrainVoice` has a fixed array of 64 grains. `Budget` counts the grains alive across all voices and refuses new ones past its limit.
- Grain sources are `Clip` (a region of a sample) and `LiveBuffer` (a circular buffer with freeze). Grains are placed so the whole read stays inside the readable span, so a grain is never cut short or reads outside its region.
- Windows are Hann, Tukey, trapezoid and exponential, as 1025-entry tables. Pitch can be quantised to a 12-note mask. Density and size can lock to the transport's frame timeline (`TempoSync`), and synced grains start on exact frames whatever the period size.
- Pads play it through `PlayMode::Granular` in `mpfree-sampler`. Each voice slot has its own `GrainVoice`, and one `Budget` (`GRAIN_BUDGET`, 256) is shared by all of them. A region keeps its grain settings in every play mode. Tune adds to the grain pitch, reverse makes every grain play backwards, and the pad's envelope, filter, pan, chokes and polyphony apply as for any other voice. Lifting the pad stops new grains and releases.
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
