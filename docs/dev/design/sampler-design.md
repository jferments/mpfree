# Sampler design

**Status:** proposal, 2026-11-10, for the owner's review. No code yet. Research behind it: [samplers: what people like, what they don't, and what to borrow](../research/sampler-landscape.md).

Companions: the [feature map](sampler-features.md) (every MPC sampler feature rebuilt with a better UI, features from other samplers, and what makes a sampler great for production and performance) and the [architecture](sampler-architecture.md) (crates, threads, data flow, real-time rules, files). Mockups of the screens and the sampler skill tree are in the [mpfree-claude repo](https://github.com/jferments/mpfree-claude/tree/main/sampler).

The sampler is the core of the MPC. This document proposes how MPFree records sound, chops and edits it, puts it on pads and plays it back, including granular tools. It builds on what exists: the pad player in `rust/mpfree-app` (kits on pads, a `SCHED_FIFO` audio thread, direct ALSA playback), the `mpfree-ui` toolkit ([UI architecture](ui-architecture.md)), the host-only `mpfree-sampler` crate, and the decisions already made: direct ALSA ([ADR-0002](../decisions/adr-0002-direct-alsa-audio.md)), MIT licence, a GUI-first, modular UI, one design for beatmakers and performers.

## Goals

1. **Never lose the moment.** The input and MPFree's own output are always being kept in a rolling buffer, so "sample what just happened" is one press.
2. **Sample, chop and play in three presses or fewer**, without touching the screen if you don't want to.
3. **One sampler, many modes.** One-shot, loop, slices, phrase, warp and granular are settings on the same object, not separate machines in separate screens.
4. **Edits are recipes.** Trims, slices, loops, fades, gain, warp and grain settings are stored with the sample and saved with it. The recorded audio is never changed. Baking ("render") is explicit and undoable.
5. **Every control has a home on the hardware.** Pads audition and pick, Q-Links move points and parameters, the wheel zooms and scrolls, the screen shows everything and can do everything.
6. **Fix the known MPC complaints:** loop points separate from the end (keep the release tail), no audition time limit, zoom that stays where you are, markers that don't cover the waveform, save from anywhere, names shown before anything is deleted, more layers.

## The model

```text
Source  ──►  Region (a recipe on part of a source)  ──►  Layer  ──►  Pad
  │             start, end, loop start/end,               velocity      bank A-H, pads 1-16
  │             crossfade, fades, gain, reverse,          range,        mode, choke and mute
  │             tune, warp, grain settings                round robin   groups, polyphony,
  │                                                                     envelope, filter
  └── Chop sets (named lists of slice points; a source can have several)
```

- **Source.** Audio exactly as recorded or imported, never modified. Identified by a content hash, so the same audio is stored once even if many pads use it. Stored in the project's `samples/` folder as WAV (24-bit or 32-bit float) or FLAC. Each source has a cache file next to it with its analysis: the waveform peak pyramid, transients, estimated tempo and root pitch. The cache can always be rebuilt.
- **Chop set.** A named list of slice points on a source ("threshold 40", "16 steps", "my chop"). Making a new chop doesn't destroy the old one; that is how re-chopping stays cheap.
- **Region.** The recipe: which part of which source, plus everything that changes how it sounds. A slice is a region whose start and end come from a chop set. Loop start and end are separate from start and end, so a sustained loop can release into the original tail. Loop crossfade, fade in and out, gain, reverse, fine tune and a playback mode live here too.
- **Layer.** A region with a velocity range, a round-robin or random group, and its own level, pan and tune. A pad has up to 16 layers (the MPC has 8; the number is a constant, sized by measurement).
- **Pad.** Up to 128 pads (8 banks of 16). Each pad has a trigger mode (one-shot, note-on/gate, toggle), polyphony (mono, poly, or a voice limit), choke and mute groups (closed hat chokes open hat), an amp envelope, a filter, pitch, and the 16 Levels settings.
- **Program.** A set of pads (a drum kit) or a keygroup (regions spread across MIDI notes for melodic playing). Both are views of the same layer data.

**Undo.** Every change is a command in a per-project log. Undo and redo walk the log; nothing is lost until the project is closed. The log is saved with the project, so undo survives a crash or a reboot.

**Project format.** A project is a folder: `project.toml` (plain text, versioned, written atomically: write a new file, then rename), `samples/` (sources and their caches), `undo.log`. It diffs in git and survives a power cut. Import of MPC projects (`.xpj`, `.xpm`) can come later for owners' own projects.

## Recording

### Where sound comes from

| Source | What it is | Status |
|---|---|---|
| Line input | The MPC One's stereo input, through the ACVA capture PCM (stereo, S32_LE, 44.1 kHz, 128-frame periods, observed) | New: `mpfree-app` opens playback only. Capture is the first piece of new on-device work |
| Resample | MPFree's own master output, or one pad, program or track before or after its effects | Engine-internal, no hardware risk |
| Import | WAV, AIFF, FLAC (and MP3, AAC, Ogg as an optional feature) from a USB stick, SD card or over Ethernet | File I/O only |
| USB audio | A class-compliant interface on the USB-A port | Unverified; later |

The MPC One has no input preamp or gain control in software (the codec is pin-controlled, no mixer). The record screen shows the level honestly and offers digital gain after recording, not before.

### The always-on buffer ("skip back")

The audio thread copies every period of input and master output into two rolling buffers in RAM. At 44.1 kHz stereo 32-bit float, one minute is about 21 MB, so keeping the last 2 minutes of each costs about 85 MB of the roughly 1.4 GB MPFree can use for audio. Pressing **Sample What Just Happened** (proposed: SHIFT + a record button, to be captured) turns the last N seconds or the last N bars into a new source. With the sequencer running, "last 4 bars" is cut exactly on the bar lines.

### Arming a recording

| Mode | Starts | Stops | Notes |
|---|---|---|---|
| Manual | Press record | Press stop | |
| Threshold | Input crosses a level | Press stop, silence for N seconds, or a length | Pre-roll comes from the rolling buffer, so the attack is never lost (the MPC's pre-record, with no setting needed) |
| In time | Next bar after a count-in | After N bars | Sets the source's tempo and makes a perfect loop |
| Retrospective | Already happened | Now | The always-on buffer, above |

### How it flows through the engine

```text
ALSA capture (period of 128 frames) ─► audio thread ─► rolling buffers (RAM, fixed size, never allocate)
                                              │
                                              └─► lock-free ring (rtrb) ─► writer thread (normal priority, core 1)
                                                                              ├─► source in the RAM pool
                                                                              └─► WAV on disk, in chunks, for long takes
```

The audio thread never allocates, locks, logs or touches files; it only copies into pre-sized buffers and a lock-free ring. A writer thread at normal priority does the rest. Long takes (more than a few minutes) go to disk as they are recorded, like the Blackbox, so a take isn't limited by RAM.

**Device safety.** Recording only writes files. On the 3.10 test build every path is in RAM and comes back to the workstation over SSH, exactly like today's results; storage hashes before and after the round confirm nothing else changed. When MPFree runs from a stick, recordings go to the stick or an SD card, never to the eMMC's system, boot or Akai partitions. The existing test that lists every place the program opens a file for writing (`test_the_program_writes_only_where_reviewed`) covers the new paths.

### After recording

The take opens in the editor with suggestions ready: the detected tempo, a proposed chop, a trim of leading and trailing silence, and peak/RMS level. Nothing is applied until you accept it.

## Chopping and editing

### The edit screen

Built from `mpfree-ui` widgets, so the layout is one string and can be rearranged freely. A first layout:

```text
col(header:60, overview:64, wave:*, row(params:*, slices:380):210, hint:26)
```

- **Overview:** the whole source, with the visible part highlighted, slices as coloured bands, and the playhead.
- **Wave:** the zoomed view. Slice markers are thin lines with a handle on the edge of the view, never on top of the waveform. Zoom stays centred on the selected marker or the playhead, whichever was touched last.
- **Params:** the selected region's settings, as faders and lists.
- **Slices:** the slice list, each row coloured like its pad.

**New widgets** for `mpfree-ui`: `Waveform` (detail view with markers, selection, loop and playhead), `WaveOverview`, `MarkerRuler`, `XYPad` (for granular), `Meter` (input level with peak hold and clip lamp). They follow the existing rules: draw into their rectangle, report touches as events, never talk to the engine.

### Controls

| Control | In the editor |
|---|---|
| Touch | Drag to scroll, pinch to zoom, tap to select or place a marker (in Manual mode), drag a marker by its handle |
| Wheel | Picks a row of the settings beside the editor; - / + step it; the wheel's push opens the row for the wheel (layout spec, section 6). SHIFT + - / + zooms; SHIFT + wheel is the next pad with a sound |
| Q-Link 1-4 | Start, end, loop start, loop end of the selected region (or the selected marker in Chop) |
| SHIFT + Q-Link | Fine adjustment, down to one sample |
| Pads | Play the slice on that pad. In Chop, hitting a pad selects that slice for editing |
| Pads while a sample plays (live chop) | Each hit drops a marker at that moment, quantised to the nearest transient or zero crossing if snap is on. Fast, musical, and only possible on hardware with pads |
| Undo / redo buttons | The command log |

**Snap:** to zero crossings (on by default, avoids clicks), to transients, to the beat grid, or off.

### Chop modes

| Mode | How slices are made | Good for |
|---|---|---|
| Transient | Spectral-flux onset detection with a sensitivity control (more or fewer slices as you turn it) | Drum breaks |
| Grid | N equal slices | Loops cut to length |
| Beat | Slices on beats or divisions from the tempo (detected or typed) | Loops at a known tempo |
| Manual and live | Tap on the screen or on pads while it plays | Melodic material, vocals |
| Silence | Split where the sound drops below a level | Sample packs recorded as one take, one-shot chains |

Slices can be joined, split, moved, renamed and recoloured. Up to 128 slices per chop set.

### Processing

All of these change the region's recipe, not the audio, and play back in real time:

trim, fade in and out, gain and normalise, reverse, tune (semitones and cents), loop with crossfade, and "vintage" colour (bit depth and sample-rate reduction).

These need heavy computing, so they make a new source in the background (the old one stays, so it is undoable): time stretch to a tempo (Signalsmith Stretch, high quality), pitch shift without changing length, rendering a region with its effects, and resampling at a new rate.

### Converting a chop

From the chop screen, one press each:

- **To pads:** slices go onto consecutive pads, from the selected pad or pad 1 of a new kit. Pad colours follow the slice colours.
- **To keys:** slices go onto consecutive MIDI notes in a keygroup.
- **To a phrase:** a pattern that replays the slices in their original order and timing, so the loop plays back intact but each hit can be moved, muted, re-pitched or swapped (the MPC5000's "new sequence" and REX-style playback). Change the tempo and the slices follow it without stretching.
- **To files:** each slice as its own WAV.

## Assigning to pads

- **Hit to assign.** Select a source, slice or region anywhere (browser, editor, the skip-back buffer), then hold the assign button (to be chosen) and hit a pad. The same gesture works for every kind of sound.
- **Layers.** Hitting an occupied pad while holding SHIFT adds a layer instead of replacing. Layers split by velocity, cycle in order (round robin) or pick at random.
- **Groups.** Choke groups (one pad stops another) and mute groups, set per pad.
- **Full Level and 16 Levels.** Full Level plays every hit at maximum velocity (the owner already wants this button working). 16 Levels spreads the selected pad's sound across all 16 pads by velocity, tune, filter cutoff, decay or slice start, as on the MPC.
- **Screen pads.** The pad grid on the screen shows each pad's slice colour and a tiny waveform. A tap plays it, a long press opens its region in the editor.

## Playback

### The voice

A fixed pool of voices is made at start-up; nothing is allocated while playing. Each voice holds:

- a reference to its region's audio (freed off the audio thread, with `basedrop`, so the audio thread never calls `free`),
- a position as a 64-bit fixed-point number and an increment (pitch),
- the interpolation mode: linear for cheap voices, 4-point Hermite by default, windowed sinc only when rendering offline,
- an amp envelope (attack, hold, decay, sustain, release), a state-variable filter with its own envelope, pan and gain,
- a short fade on steal and on choke (the existing `mpfree-sampler` already uses 64 and 32 frames) so nothing clicks.

The pool size starts at 64 voices and is set by measurement on the RK3288.

### Playback modes (per region)

| Mode | What it does |
|---|---|
| One-shot | Plays to the end whatever the pad does |
| Note-on (gate) | Plays while the pad is held, releases when it lifts |
| Loop | Loops between the loop points while held, then plays the tail after release |
| Warp | Plays at the project tempo. Uses a pre-stretched copy made in the background (high quality); while it is being made, or for live tempo changes, the granular engine stretches in real time |
| Granular | See below |

### Memory

Sources are held in RAM as 32-bit float stereo, about 21 MB a minute; 1.2 GB is about 57 minutes. A 16-bit source can be kept as 16-bit and converted per sample read, halving its memory; the cost of the conversion is measured before deciding. Long sources that don't fit stream from disk: the first second is kept in RAM so a hit starts at once, and a reader thread on core 1 keeps each streaming voice a few hundred milliseconds ahead. Disk streaming is a later step; the first versions keep everything in RAM.

The MPC One's process limit is 32-bit (about 3 GB of address space) and the board has about 2 GB of RAM. The earlier plan budgets about 1.4 GB for samples once Akai's app is not running; the real figure is measured on the test build.

### Threads

| Thread | Priority | Core | Does |
|---|---|---|---|
| Audio | `SCHED_FIFO` 70 | 3 (2 later, if tracks are split across two cores) | Reads capture, mixes voices, writes playback, copies into the rolling buffers |
| Writer, loader, analysis | Normal | 1 | Recording to disk, loading and decoding, peak pyramids, onset detection, stretching |
| UI | Normal | 0 or 1 | Screen, touch, controls |

Commands go to the audio thread through lock-free rings; finished sources are handed over by pointer. This is the same shape as the pad player's channels today.

## Granular

Granular is a playback mode of any region and an effect on any live signal, using one engine.

### The grain engine

Each granular voice spawns grains from its region (or from a live circular buffer). Per grain: a start position, a length, a pitch, a direction, a pan and a window. Each output sample of a grain is one interpolated read and one multiply by the window, so the cost scales with the number of grains playing.

| Parameter | Range (first guess) | Default control |
|---|---|---|
| Position | 0-100 % of the region | Q-Link 1, or X on the touch pad |
| Spray | 0-100 % | SHIFT + Q-Link 1 |
| Size | 2 ms to 2 s, or a note length when synced | Q-Link 2, or Y on the touch pad |
| Density | 1-200 grains/s, or synced divisions | Q-Link 3 |
| Pitch | ±36 semitones, optional scale quantise | Q-Link 4, or the pads/keys in keygroup mode |
| Pitch spray | 0-12 semitones | SHIFT + Q-Link 4 |
| Scan speed | -2 to +2 (1 = original speed, 0 = frozen) | Wheel |
| Window | Hann, Tukey, trapezoid, exponential | List |
| Reverse chance | 0-100 % | Fader |
| Stereo spread | 0-100 % | Fader |
| Freeze (live mode) | On or off | A button, or hold a pad |

**Pad pressure** (the pads send polyphonic aftertouch) can drive density, size or position per pad, so pressing harder thickens the cloud.

**The granular screen** shows the region with each grain drawn as a short bar where it is reading, so you can see the cloud move. An `XYPad` widget covers position and size; four faders show the Q-Link parameters.

### Granular as an effect

A circular buffer (a few seconds) on the input, a pad, a program or the master. The same grain engine reads from it. Freeze stops writing and keeps the current sound. This is the Clouds or S-4 Mosaic idea. Mutable Instruments' Clouds and Beads firmware is MIT licensed, so MPFree may study it and port ideas or code with attribution; confirm the licence in the files before copying anything.

### Cost

A rough estimate, to be measured: 64 grains at 44.1 kHz stereo is about 5.6 million grain-samples a second. With a Hermite read and a table window that is a few tens of millions of operations a second, a fraction of one Cortex-A17 core at 1.6 GHz. The audio thread's budget is 2.9 ms per period; M7 measured 66 µs for the old sampler test. A global grain limit protects the period: past it, new grains are skipped, never queued.

## Analysis and waveforms

All analysis runs on the worker thread, results go into the source's cache file.

| What | How |
|---|---|
| Waveform peaks | A pyramid of min, max and RMS per channel, stored as 16-bit values, at block sizes 64, 256, 1024, ... frames. The `Waveform` widget picks the level nearest one block per pixel column, so drawing a 1280-pixel view is 1280 lookups whatever the zoom. Below one sample per pixel it draws the samples themselves. About 3 % of the audio's size |
| Transients | Spectral flux from a short FFT (`realfft`), an adaptive threshold, and a sensitivity knob |
| Tempo | Autocorrelation of the onset curve, with the result offered, not forced |
| Root pitch | YIN (`pitch-detection` or our own) on the steady part |
| Zero crossings | Found on demand around a marker |

Drawing in software today: a full waveform view is a few thousand rectangles, and the playhead redraws only a narrow column. When the GLES canvas arrives, the peak pyramid becomes a texture.

## Libraries

See the [library table](../research/sampler-landscape.md#libraries) for licences. In short: `hound`, `claxon` and optionally `flacenc` and `symphonia` for files; `rubato` for rate conversion; `realfft` for analysis; `signalsmith-stretch` for high-quality stretching; `rtrb` and `basedrop` for real-time plumbing; our own code for the waveform, voices, filters, envelopes and the grain engine. Every crate is pinned in `Cargo.lock` and builds with `--locked`. Signalsmith Stretch is C++, so it needs `arm-linux-gnueabihf-g++` on the workstation; it is the only C++ in the plan, and it can wait until the warp step.

## Crates

| Crate | Contents |
|---|---|
| `mpfree-sampler` (grows from today's prototype) | Source, region, layer, pad, program; the voice pool; playback modes; no I/O in the real-time path |
| `mpfree-grain` (new) | The grain engine, used by sampler voices and by the granular effect |
| `mpfree-analysis` (new) | Peak pyramid, onsets, tempo, pitch; workstation-testable, no real-time use |
| `mpfree-audio` (new, split out of `mpfree-app`'s `alsa.rs`) | ALSA playback and capture behind the `AudioDevice` trait from ADR-0002 |
| `mpfree-ui` | New widgets: `Waveform`, `WaveOverview`, `MarkerRuler`, `XYPad`, `Meter` |

## Steps

Each step is tested on the workstation first (unit tests, offline renders, preview PNGs of the screens), and only the steps marked "MPC" need a round on the device, run on the 3.10 test build with storage hashes before and after.

| Step | What | Where |
|---|---|---|
| S1 | The model (source, region, layer, pad) and the voice pool with modes, envelopes and the filter; offline renders compared against reference files; no-allocation check on the render path | Workstation, CI |
| S2 | Peak pyramid and the `Waveform` and `WaveOverview` widgets, with preview screenshots of the edit screen | Workstation, CI |
| S3 | Transient, grid, beat and silence chop; chop to pads; play chops from the factory demo kits' loops in the pad player | Workstation, then MPC |
| S4 | ALSA capture: input meter, threshold and manual recording, the rolling buffer, resample; record, chop, play in one session | MPC |
| S5 | The edit screen with touch, Q-Links, the wheel and live chop; undo; save and load a project folder (on the stick or in RAM) | Workstation, then MPC |
| S6 | Granular voice and the granular effect; grain and voice counts measured on the device | Workstation, then MPC |
| S7 | Warp (background stretching), chop to phrase, keygroups, 16 Levels | Workstation, then MPC |
| S8 | Disk streaming for long sources; MP3 and other imports; MPC project import | Later |

## Open questions

- Which front-panel buttons open the sampler, record and assign. Most button numbers are not captured yet; capturing them all once is a small read-only step that also serves the owner's other requests (Menu, Shift, Latch, Full Level).
- The real bit depth, noise floor and clip level of the line input.
- How many voices and grains fit in a period on the RK3288, with the UI running.
- Whether 16-bit storage is worth its conversion cost.
- Where projects live once MPFree boots from a stick: on the stick, on an SD card, or both.
