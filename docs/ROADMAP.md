# Roadmap and acceptance gates

This is a proposal, not a promise of parity or a timeline.

## Incremental host-only progress (not Phase 1/2 MPC acceptance)

- [x] **M0:** deterministic C++ sample-frame click and 44.1-kHz stereo S32_LE WAV; owner confirmed audible desktop playback.
- [x] **M1:** Rust-first metronome with native debug/release tests, three-tempo **zero-difference** WAV parity against C++, and ARMv7 EABI5 hard-float CI **cross-build**. This is not a runtime compatibility or MPC sound-device test.
- [x] **M2:** Rust 16-step pattern compiler, timestamped note events, synthetic kick/snare/hat source bank and bounded 16-voice playback; desktop-only generated groove WAV and CI validation.
- [x] **M3 host:** Rust `MPF1` framed PCM codec and bounded playout model ported from Python (no network sockets); Rust WAV PCM16/24/32/float32 sampler (note mapping, release envelope, 16-voice limit and 64 MiB bank cap); CI unit/synthetic WAV smoke.
- [x] **M3 safety tools:** read-only ABI/loader/library inspection `--abi`, read-only factory service/device ownership `--handoff-preflight`, and host-only GLIBC/ELF comparator.
- [x] **M3/M4 MPC observations:** owner successfully ran the revised `--abi` and `--rtthreads`: embedded GNU libc release banner explicitly confirms **2.39**; the factory has **38 threads across cores 0–3**, including audio processing on CPU3, ~1,016 MiB locked/1,019 MiB resident memory and exclusive ACVA PCM ownership. Core affinity is not CPU utilization.
- [x] **M4 host-only safety:** no-unsafe pure Rust handoff/watchdog state-machine simulation with gates for recovery/owner consent; CI tests crash/cancel/timeout/restore failure. **No actual systemd or device-control backend exists.**
- [ ] **M4 MPC next passive probes:** re-run fixed `--rtthreads` to read per-task SCHED policy/RT priority from `/proc/task/stat`; run two-second read-only `--capacity` at idle and under ordinary playback to quantify real CPU/memory pressure.
- [ ] **M4 actual recovery:** independent eMMC/boot restore **unproven**, real stock unit stop/restart/handoff prohibited until separately owner-approved and simulated.
- [x] **M5 host build/tooling:** first standalone Rust `mpfree-device-smoke` (4,096 frames in memory, zero device opens), exact-on-device ABI/ELF preflight, opt-in `/run`-only execution harness with hash, timeout and cleanup. **Host CI checks are required; execution on physical MPC remains owner-run and unverified.**
- [ ] **M5 actual ARM execution:** owner runs `--ram-smoke-check` then explicitly opts into `--ram-smoke` after reviewing reports; verify factory app remains active, no persistent writes, and clean exit.
- [ ] **M6 independent MPFree headless app:** live typed engine/transport/null-audio adapter + bounded event loop alongside stock MPC; later exclusive audio resource handoff remains blocked on independent recovery.
- [x] Read-only mapping of Rockchip hardware, ACVA ALSA buffer settings, MIDI graph, storage layout, 1-Gbit Ethernet PHY, USB 2.0 high-speed port and Linux DWC2 device controller.
- [ ] Target-image-compatible ARMv7 sysroot and tested reversible **nonpersistent** custom executable launch.
- [ ] Proven independent unbootable-MPC recovery from verified backups, plus volatile watchdog and tested service restoration (hard gate).
- [ ] Demonstrate controller input and safe ALSA playback **on actual MPC** after verifying factory service handoff and recovery.
- [ ] Port host-only Python network-audio datagram model to Rust; add bounded real packet pacing/clock-drift simulation before any MPC audio ingress.
- [ ] Implement user-provided sample importer and genuine sequencer editing, projects, MIDI and hardware HAL.

These engineering milestones deliberately do **not** mark the Phase 1 or Phase 2 gates complete. No new software has been installed onto the MPC, and the original `acvs.service` still owns PCM and display resources. For source and tests see [Rust M1](engineering/RUST_M1_PARITY.md) and [Rust M2](engineering/RUST_M2_GROOVE.md).

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

- Engine language: **DECIDED — Rust-first**, retaining C/C++ compatibility adapters for established instruments, DSP and plugin hosting; see [ADR-0001](engineering/ADR-0001-RUST-FIRST-HYBRID.md). Hardware real-time performance remains unqualified.
- UI: Qt Quick vs SDL or other lightweight renderer.
- Audio: ALSA direct vs JACK/PipeWire (measure on device).
- Licensing: GPL-3.0-or-later, MPL-2.0, or other approved FOSS license.
- Plugin API: LV2, CLAP, custom internal graph, or combination; hardware and licensing constraints apply.
