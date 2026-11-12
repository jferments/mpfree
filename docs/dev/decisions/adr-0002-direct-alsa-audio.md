# ADR-0002: Direct ALSA audio backend on the MPC One (ACCEPTED)

**Date:** 2026-11-09
**Status:** **ACCEPTED (owner decision, 2026-11-09)**. On the MPC One, the MPFree engine owns the ACVA PCM directly through ALSA. No JACK or PipeWire daemon runs in the on-device audio path. Sound servers remain supported as development backends and as a possible later opt-in mode.
**Basis:** [audio runtime survey](../hardware/audio-runtime.md), [ABI/handoff survey](../hardware/abi-handoff-survey.md), [ADR-0001](adr-0001-rust-first.md).

## Context

| Item | Observed |
| --- | --- |
| Audio device | ALSA `ACVA`, card 1 device 0, stereo `RW_INTERLEAVED` `S32_LE`, 44,100 Hz |
| Factory stream settings | 128-frame period (~2.902 ms); playback buffer 384 frames (3 periods), capture buffer 256 frames (2 periods) |
| Stream start | Boundary-sized `start_threshold` values: the factory app starts the stream itself |
| Kernel | Linux 6.6.95 PREEMPT_RT, `isolcpus=2-3` |
| CPU / RAM | Four ARMv7 cores (RK3288), ~2 GiB RAM, no swap |
| Userspace | ARM EABI5 hard-float; no JACK or PipeWire in the factory audio path |

MPFree is a single-purpose appliance. Mixing, tracks, buses and effects live inside one engine process, so the main benefit of a sound server (routing audio between many applications) does not apply. MPFree must serve both beatmakers (as many voices and effects as the CPU allows) and live performers (no dropouts, low and stable latency).

## Decision

1. **Direct ALSA on the device.** A Rust ALSA adapter in the engine opens the ACVA PCM, sets hardware and software parameters explicitly (starting from the factory 128-frame period, 3-period playback and 2-period capture profile), links capture and playback, and controls the stream start.
2. **One realtime audio thread** runs at `SCHED_FIFO`, pinned to an isolated core (2 or 3), with memory locked and pre-faulted, and no allocation, locking or I/O in the callback. Worker threads for parallel DSP may use the other isolated core.
3. **An `AudioDevice` trait** separates the engine from the backend. Planned implementations:
   - `AlsaDirect`: the on-device backend.
   - `Null`: the existing headless/timing backend (see [M6](../milestones/m06-headless-null-audio.md)).
   - CPAL or JACK: desktop development and testing.
   The engine must not depend on ALSA-specific behavior outside the `AlsaDirect` implementation.
4. **Sound servers stay possible later.** An optional "JACK mode" (or PipeWire) on the device, where the engine runs as a client behind the same trait, may be added if users want to run other Linux audio software beside MPFree or use extra USB audio interfaces that run on their own clocks. It is not part of the default appliance.

## Rationale

- **Latency and determinism:** a sound server adds a process, extra thread wakeups every ~2.9 ms period, and its own buffering policy. Direct ALSA lets MPFree choose period, buffer, start and linking behavior exactly.
- **Resources:** no daemon (or WirePlumber and a session bus for PipeWire) uses RAM and CPU that could go to voices and effects.
- **Reliability:** one fewer process that can fail and silence the device.
- **Cross-building:** nothing extra to build and qualify for ARMv7 hard-float.
- **Licensing was not decisive:** PipeWire is MIT and libjack is LGPL; both would be compatible with an MIT project.

## Costs and accepted trade-offs

- MPFree must handle xrun recovery, stream restart and capture/playback sync itself.
- Using extra USB audio interfaces alongside ACVA will need MPFree-owned clock-drift handling (resampling), or the later optional sound-server mode.
- Other Linux audio apps cannot share the card while MPFree owns it.

## Qualification gate

This decision rests on reasoning, not yet on measurement. Before the backend is considered qualified, after the stock application safely releases ACVA (see [reversible handoff](../research/reversible-handoff.md)):

- Enumerate ACVA's supported rates, formats and period limits.
- Run an xrun soak test (at least one hour) under representative DSP load at 128×3, and also try 64-frame periods.
- Measure round-trip latency with a physical loopback.
- Record callback execution time against the ~2.9 ms period budget.

If direct ALSA fails these tests where a sound server would pass, revisit this ADR with the measurements.
