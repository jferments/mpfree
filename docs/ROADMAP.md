# Roadmap and acceptance gates

This is a proposal, not a promise of parity or a timeline.

## Phase 0 — Evidence and risk reduction
- [ ] Record exact target model and factory firmware version.
- [ ] Review licensing and choose a repository license.
- [ ] Inventory kernel, audio devices, controller MIDI endpoints, touchscreen, graphics, storage, and recovery options.
- [ ] Identify a reversible, offline, non-destructive way to launch a custom userspace application.
- [ ] Define a desktop simulator for the hardware inputs and display.

**Gate:** We can document an accurate hardware map and a safe way back to the factory application.

## Phase 1 — Hello MPC
- [ ] Build a native ARM userspace executable.
- [ ] Display a custom full-screen interface.
- [ ] Receive at least one pad/button/encoder event.
- [ ] Generate and output a metronome click through a low-latency audio backend.
- [ ] Measure and log audio latency and xruns without blocking the audio callback.

**Gate:** A reproducible, non-destructive demonstration on real MPC One hardware.

## Phase 2 — Playable instrument
- [ ] Sample loading and pad mapping.
- [ ] Velocity response and choke groups.
- [ ] Transport and tempo.
- [ ] MIDI input/output and sync.
- [ ] Project save/load, undo, and crash recovery.

**Gate:** Record, save, reload, and perform a basic beat without a computer.

## Phase 3 — Distinctive sequencer
- [ ] Per-track polymeter and independent clip lengths.
- [ ] Microtiming, conditional triggers, probability, ratchets, parameter locks.
- [ ] Fast step editing using pads + encoders; touchscreen is optional for core operations.
- [ ] Clip launcher plus linear arranger with easy conversion between them.
- [ ] Nondestructive automation and modulation routing.

**Gate:** Produce a multi-section composition that would be awkward in a conventional MPC workflow.

## Phase 4 — Production workstation
- [ ] Audio tracks, overdubbing, resampling, and slicing.
- [ ] Flexible buses, sends, effects chains, sidechain routing.
- [ ] Instrument/effect plugin architecture.
- [ ] Browser with tags, auditioning, and content provenance.
- [ ] Export stems, MIDI, and project archive.

## Phase 5 — Compatibility and distribution
- [ ] Optional local import of supported user-owned content.
- [ ] Assess factory project import and legally compatible native plugin hosting.
- [ ] Package signed/reproducible releases with backup/recovery documentation.
- [ ] Test on MPC One, then separately evaluate One+ and other models.

## Prioritization principles

Prioritize low menu depth, immediate physical control, reliable audio, reversible operations, and expressive sequencing. Do not let 'all factory features' delay a useful, independently playable first release.

## Open decisions

- Engine: Rust vs C++ (benchmark and review real-time guarantees).
- UI: Qt Quick vs SDL or other lightweight renderer.
- Audio: ALSA direct vs JACK/PipeWire (measure on device).
- Licensing: GPL-3.0-or-later, MPL-2.0, or other approved FOSS license.
- Plugin API: LV2, CLAP, custom internal graph, or combination; hardware and licensing constraints apply.
