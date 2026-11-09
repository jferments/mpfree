# mpfree architecture (proposal, v0)

## Mission

Build a community-owned, open-source standalone music-production application for Akai MPC One hardware. The initial target is a **userspace replacement**, not a replacement Linux kernel or a firmware image. No device compatibility or factory-plugin compatibility is claimed yet.

Design goals:
- Musical immediacy: pads, transport, and encoders work without menu diving.
- Independent, composable tracks and clips; avoid forcing users into a single track/program/sequence hierarchy.
- Sample-accurate sequencing with per-track lengths, polymeter, probability, conditional triggers, microtiming, parameter locks, automation, and a linear arranger.
- Sampling/chopping, MIDI and CV sequencing, audio recording, mixing, effects, synthesis, project saving, and eventual compatibility/import where permitted.
- Offline-first, repairable, reproducible builds, user-controlled data, no telemetry by default.
- Non-destructive installation and easy return to factory behavior.

## Proposed layers

```
Touchscreen UI + hardware workflow (candidate: Qt Quick/QML or SDL)
                       |
        Application model / commands
                       |
    Project model + undo/redo + persistence
                       |
   Sequencer + sample engine + DSP + mixer
                       |
    Hardware abstraction (audio/MIDI/pads/encoders/display/CV)
                       |
          Existing Linux userspace/kernel
                       |
                   MPC One
```

**Decisions not yet made:** C++ vs Rust for the real-time engine, Qt vs SDL for the UI, ALSA direct vs JACK/PipeWire, plugin ABI, sample-streaming strategy, boot integration, and compatibility with factory content. All require measurement on the target device.

### Real-time boundary

Audio callback: bounded work only; no file I/O, UI calls, dynamic memory allocation, logging, network access, or contended locks. Prefer preallocated buffers, immutable snapshots or lock-free queues, and sample-frame timestamps. Keep UI rendering, filesystem access, indexing, and plugin management off the audio thread.

Use a deterministic sequencer model that can be unit-tested on a desktop before connecting to audio. Validate round-trip MIDI timing, audio latency, xruns, and shutdown/recovery on real hardware.

### Hardware adapter

Expose a versioned interface for:
- Audio input/output and latency measurement
- Pad note/velocity/aftertouch (if supported), buttons, transport, encoders, LEDs
- Touchscreen and display
- DIN/USB MIDI, MIDI clock, transport sync
- CV/gate ports (subject to verification)
- External storage and safe power-off

Do **not** assume that controller messages documented for MPC Live apply unchanged to MPC One or One+ firmware. Treat all device mappings as discoverable/testable.

### Project model (conceptual)

```
Project
  ├── Assets (samples, presets, external references)
  ├── Tracks (audio, drum, instrument, MIDI, CV)
  │     ├── Devices / DSP graph
  │     ├── Clips (independent lengths, time bases, note events)
  │     └── Automation / parameter locks / modulation
  ├── Scenes / clip launcher
  ├── Arrangement (timeline referencing clips and audio regions)
  ├── Routing / mixer / buses / sends
  └── Undo journal / versioned metadata
```

Clips and arrangements should reference assets by stable IDs, not fragile absolute paths. Save atomically with versioned migrations; autosave must not block audio. Avoid vendor-specific formats as the only storage option.

## Development and test strategy

1. Desktop simulation of transport, pads, knobs, screen, and a basic sequencer.
2. Read-only inventory of an MPC One: kernel, architecture, ALSA devices, controller ports, display and input devices. Preserve logs; redact identifiers before sharing.
3. Hardware proof of concept: display one page, read one pad, play a click, change tempo, stop cleanly. Do not replace the factory boot process.
4. Build a minimal playable sampler with stable audio and MIDI.
5. Implement high-impact sequencing and arrangement workflows.
6. Expand recording, DSP, synths, plugin hosting, compatibility, and advanced UI.
7. Only consider alternative boot integration after a documented backup and recovery path exists.

## Explicit non-goals for v0

- Shipping proprietary Akai/AIR binaries, factory sample packs, presets, keys, or firmware images.
- Bypassing license checks or activation.
- Replacing bootloader/kernel before basic userspace is proven.
- Claiming 100% MPC 2/3 feature parity.
- Shipping an untested installer that can overwrite factory partitions.

## Related research

- https://github.com/TheKikGen/MPC-LiveXplore
- https://github.com/TheKikGen/MPCLiveXplore-libs

These are third-party exploratory projects, not endorsed hardware APIs. Review their licenses before copying any code.
