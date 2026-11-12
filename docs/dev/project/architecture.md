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
Touchscreen UI + hardware workflow (mpfree-ui; see ui-architecture.md)
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

**Language choice ACCEPTED:** [ADR-0001](../decisions/adr-0001-rust-first.md) establishes **Rust-first with narrow C/C++ compatibility modules**. [Rust M1](../milestones/m01-rust-metronome-parity.md) has a working native core and desktop WAV renderer with C++ parity and ARMv7 hard-float cross-build checks. The C++ M0 engine is retained as a tested reference, while az0x binary execution is not yet validated. Since decided: direct ALSA on the device ([ADR-0002](../decisions/adr-0002-direct-alsa-audio.md)) and the UI (SDL3 and a custom Rust UI on OpenGL ES 3, started as [`mpfree-ui`](../design/ui-architecture.md)). Still undecided: plugin ABI, sample streaming implementation, boot integration, and factory-content import details.

**Audio backend ACCEPTED:** [ADR-0002](../decisions/adr-0002-direct-alsa-audio.md) has the engine own the ACVA PCM directly through ALSA, behind an `AudioDevice` trait; JACK, PipeWire and CPAL are desktop or optional backends. On-device xrun and latency qualification is pending.

### Real-time boundary

Audio callback: bounded work only; no file I/O, UI calls, dynamic memory allocation, logging, network access, or contended locks. Prefer preallocated buffers, immutable snapshots or lock-free queues, and sample-frame timestamps. Keep UI rendering, filesystem access, indexing, and plugin management off the audio thread.

Use a deterministic sequencer model that can be unit-tested on a desktop before connecting to audio. Validate round-trip MIDI timing, audio latency, xruns, and shutdown/recovery on real hardware.

### MPC One hardware findings (November 2026 verified evidence)

See [first hardware survey](../hardware/survey-2026-11-09.md) and [factory resource-ownership survey](../hardware/resource-ownership.md). These **observations** supersede earlier guesses that the target audio and GUI interfaces were unknown:

- **SoC/CPU:** Rockchip RK3288; four ARMv7 cores (NEON/VFPv4), performance governor observed at 1.608 GHz; ~2 GiB memory, zero swap.
- **Realtime kernel:** OpenEmbedded az0x 5.0.8 Linux 6.6.95 PREEMPT_RT. Neither a replacement kernel nor a supported ARM ABI is yet implemented.
- **Audio:** ALSA card 1 ACVA with one playback and one capture PCM subdevice. Both were opened by the stock factory process with zero available subdevices at the observation moment. PCM rate/format/periods remain unmeasured.
- **Controllers:** USB MIDI device `09e8:0046`, with three raw MIDI subports labelled Public/Private/MIDI Port. The stock application opens the raw-MIDI device three times. Port-to-pad/encoder/LED semantics remain unknown.
- **UI:** factory app holds `/dev/dri/card0` and evdev `event0` (ILI2117 touchscreen), `event1` (GPIO keys). Mali-T760-compatible GPU has Panfrost and a DRM render node. Connector mode is **800x1280**; physical landscape transform remains unknown.
- **Stock process:** `acvs.service`, launched via `/usr/bin/az01-launch-MPC`. Its process holds ALSA sequencer, playback, capture, controls, raw MIDI, evdev and DRM file descriptors concurrently. Do not hardcode session PID or file-descriptor numbers.
- **Persistent storage:** factory eMMC partition labels are known, and `/etc`/`/var` are persistent writable overlays despite a read-only `/`. Recovery is not yet proven.

**Implementation boundary:** the initial MPFree engine, transport and UI model must work on a workstation with a fake hardware backend. The MPC One HAL should (a) enumerate actual devices, (b) report ownership and capabilities without opening conflicting endpoints, and only later (c) acquire PCM/MIDI/KMS after a user-approved, reversible factory handoff. Never create a "startup on boot" hook during discovery, and do not stop `acvs.service` until a tested rollback/restore mechanism exists.

A useful initial resource-state model is:

```text
SURVEY_ONLY → SUPPORTED_HARDWARE_VERIFIED → RECOVERY_VERIFIED
            → CONSENTED_SESSION_TAKEOVER → RESTORE_STOCK
```

This is a **design state machine**, not a working installer. The later `CONSENTED_SESSION_TAKEOVER` stage is explicitly blocked today by missing backup/recovery proof and unknown audio/MIDI control semantics.

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
