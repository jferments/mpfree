# Native mpfree sequencer — first working component

**Hardware target:** original MPC One, factory firmware **3.6.0**. This C++20 engine component is portable desktop code and has **not yet been tested on MPC hardware**.

The first engine primitive is an independent, sample-frame-based event scheduler. It does not depend on Akai binaries, the Linux audio subsystem, any proprietary SDK, or a particular GUI framework.

## Features implemented

- Independent step lengths per track (polymetric sequencing).
- Independent steps-per-beat per track.
- Exact integer sample-frame event timestamps and per-block offsets.
- Negative and positive microtiming offsets (up to 2048 frames).
- Ratchets (up to eight substeps).
- Deterministic, seed-based probability independent of audio block boundaries.
- Every-N-clip-cycle conditional triggers.
- Four parameter locks per step, copied into scheduled events.
- Bounded output buffer with overflow reporting.
- Sorted events, no allocation/locks/I/O in the scheduling function.

## Build and test

Requirements: CMake >= 3.20, a C++20 compiler.

    cmake -S . -B build -DCMAKE_BUILD_TYPE=Debug
    cmake --build build
    ctest --test-dir build --output-on-failure

Render a synthetic 4-second demonstration of two independent rhythms:

    ./build/mpfree_demo ./build/mpfree-demo.wav

The generated WAV contains no copyrighted factory samples; it uses synthesized test clicks.

## Interface contract

The caller supplies an immutable Song, a block start timestamp, a block length in frames, and a preallocated span of Event structures. The scheduler writes events ordered by absolute frame timestamp.

**Real-time note:** The scheduler itself avoids allocations, locks, and I/O, but the overall system is not yet certified real-time. The Song/Track/Step vectors must be constructed or replaced **outside** the audio callback, and performance must be measured on the MPC One. Production code will require an immutable snapshot/update protocol and explicit handling of tempo changes, clock sync, transport seeking, and block overflow.

## Deliberately not implemented yet

- Actual MPC pad/encoder/audio device drivers.
- Audio callback, sample playback, sample streaming, synthesis, DSP graph.
- Song arrangement and clip launch UI.
- MIDI/CV output or synchronization.
- Tempo automation, swing/groove, tuplets, time-signature maps.
- Dynamic track edits during playback.
- Robust handling of extreme song lengths or pathological tempo values.
- A file format for Song serialization.

The engine will be integrated with the hardware adapter only after the device interface and recovery path are validated.
