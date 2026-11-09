# MPFree M2 — Rust step sequencer and bounded sample voices

**Status:** first host-only, offline sequencing and polyphonic-sampling prototype implemented; **no MPC audio device, no MIDI connection, no new firmware, no proprietary content**. See [accepted Rust-first architecture](ADR-0001-RUST-FIRST-HYBRID.md) and [Rust M1 parity](RUST_M1_PARITY.md).

## Motivation

M1 proved Rust can generate deterministic 44.1-kHz stereo S32_LE WAV output equal to our earlier C++ engine at three tested tempos. M2 begins replacing a fixed click with **a user-defined rhythmic pattern, absolute sample-frame event scheduling, sample playback, mixing and finite voice allocation**. It provides actual sequencer/audio-engine structure while preserving a desktop-only failure boundary.

## Implementation and ownership

The new `rust/mpfree-groove` workspace package provides a reusable library and a host-only `mpfree-groove` CLI:

| Component | Responsibilities | Real-time allocation status |
| --- | --- | --- |
| `Pattern` | 16 steps per bar, independent kick/snare/hat velocities `0..=127`, `0` = inactive | Prepared off audio thread |
| `GrooveConfig` | Explicit sample rate, BPM, bars; bounds 8–192 kHz, 20–400 BPM, 1–128 bars | Validated off audio thread |
| `compile_pattern()` | Precomputes stable, chronological `Event {frame, drum, velocity}` entries for repeated bars using absolute sample time | **Allocates Vec before render** |
| `SampleBank::new()` | Generates three synthetic one-shot drum PCM arrays: pitched kick, noisy snare, short hi-hat | **Allocates Box buffers before render** |
| `GrooveEngine` | Owns precompiled events, precomputed samples, fixed `[Voice; 16]` pool, event cursor and sample-frame transport | One owner on audio thread |
| `GrooveEngine::render()` | Trigger due events, mix active voice samples, clamp output, write planar stereo float arrays | No explicit allocation, locks, syscalls, I/O, logging or trig |
| `mpfree-groove` binary | Offline RIFF WAV serialization and CLI; S32 conversion reused from `mpfree-core` | **Outside real-time callback** |
| `Rust tests / CI` | Correct event timing, exact output length, deterministic blocks, finite voice stealing, invalid geometry and offline WAV | Host-only |

Both Rust source files contain `#![forbid(unsafe_code)]`. The crate has **no third-party dependencies** and reuses the verified `mpfree-core::float_to_s32` converter. There is no audio HAL or live socket in the crate.

### Timing model

One bar is exactly four quarter-note beats, divided into 16 steps (four steps/beat). For absolute step position `n` (zero-based):

\[
f_n = \operatorname{round}\left(\frac{n \cdot f_s \cdot 60}{4 \cdot b}\right)
\]

where:

- `f_n` = integer PCM frame at which step `n` triggers;
- `n` = absolute step index across repeated bars;
- `f_s` = sample rate in frames per second;
- `b` = BPM (quarter-note tempo);
- `4` = number of sixteenth-note steps per quarter-note beat.

This calculates *each absolute trigger position* rather than adding the same rounded integer step length. At 44.1 kHz and 120 BPM, a bar is 88,200 frames (2 seconds), and steps land ~5,512.5 frames apart, so adjacent step gaps alternate by a frame. The exact timing comes from recomputing position from absolute index and rounding, avoiding cumulative fixed-step drift.

The sample-frame event list is compiled **before** audio playback. During rendering we compare the current frame to the next event's timestamp; all same-frame events are triggered in stable Kick → Snare → Hat order. Event compilation supports arbitrary 16-step patterns by changing the steps; the CLI currently uses a preset 2-kick / 2-snare / 8-hat bar.

### Bounded memory and voice behavior

- The complete bar pattern's events are precomputed in a finite Vec, with maximum 128 bars × 16 steps × 3 trigger slots = **6,144 events**, with no event allocation in the callback.
- Synthetic drum sample buffers contain ~240 ms kick, 160 ms snare, and 90 ms hat, prepared deterministically at startup. The default 44.1-kHz sample bank is well under 200 KB; this is a prototype with no external file loader.
- `MAX_VOICES = 16`. When no inactive voice exists, a fixed round-robin victim slot is reused (voice stealing). The algorithm is deterministic but can abruptly stop an old sound; a production sampler should choose a better policy and declick/crossfade.
- Every render processes at most 16 active voice samples per output frame, plus the precompiled events due at that frame.
- Planar `&mut [f32]` buffers are supplied by the caller. A channel-length mismatch returns a typed error without advancing transport.
- The demo mixes identical left/right channels and hard-clamps ±0.98 for basic clipping prevention. This is **not** production gain staging, true peak limiting or loudness normalization.

**Real-time caveat:** code inspection indicates no allocations/locks/syscalls in `render`; this has not yet been instrumented with a custom allocator guard or tested for worst-case execution time on ARMv7. Fast host-only test success does not establish live 128-frame deadline compliance or silence-free output on MPC hardware.

## Hear the two-bar groove on ML-tower

Prerequisites: Rust/Cargo (owner reports rustc and cargo 1.97.1) and existing MPFree checkout.

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey

cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo test --manifest-path rust/Cargo.toml --workspace --release --all-targets --locked

cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-groove
./rust/target/release/mpfree-groove /tmp/mpfree-groove.wav 120 2

file /tmp/mpfree-groove.wav
xdg-open /tmp/mpfree-groove.wav
~~~

At 120 BPM, two bars last **4 seconds** and produce **176,400** PCM frames. With the preset pattern, expect **24 timed drum events** (4 kicks, 4 snares, 16 hats) and zero voice steals in normal configuration. The output WAV is 44,100 Hz stereo signed-32-bit little-endian PCM, containing **synthetically generated sounds**; none are copied from Akai.

Other short examples:

~~~bash
./rust/target/release/mpfree-groove /tmp/mpfree-groove-90bpm.wav 90 2
./rust/target/release/mpfree-groove /tmp/mpfree-groove-137bpm.wav 137.2 4
~~~

The standalone renderer is **limited to 60 seconds** per invocation to prevent accidental large WAV outputs; the library itself supports longer prepared timelines with bounded event counts. This is not yet a real-time sound engine or a user-facing arrangement editor.

## Acceptance tests

Automated Rust unit tests (debug and release) cover:

1. Basic preset produces exactly 24 events for two bars, with kicks at frames 0, 44,100, 88,200 and 132,300 (120 BPM).
2. At non-integer BPM (137.2), the beginning of bar 64 matches the absolute-time formula rather than accumulated rounded sixteenth-note steps.
3. Whole-buffer versus 128-frame-chunk rendering is **bit-exact within Rust**.
4. Rendering covers all 24 prepared events; stereo equality; PCM finite and bounded; prepared sample bytes remain below the test threshold.
5. Invalid BPM, bars and sample rates are rejected before state changes; mismatched output slice lengths do not advance transport.
6. Explicitly injecting more than 16 simultaneous voices verifies deterministic, bounded voice stealing.
7. Reset reproduces the same PCM block deterministically.

CI also validates offline WAV RIFF structure, sample rate/width, four-second duration, expected event count, nonzero audio and equal left/right PCM. The Rust groove library and executable are included in the existing ARMv7-gnueabihf CI cross-build. **Cross-building is not proof that the binary can run under az0x's installed glibc/loader.**

No workflow uploads build artifacts or touches the MPC.

## Verified GitHub CI acceptance (2026-11-09 UTC)

[GitHub Actions run 42576743311](https://github.com/jferments/mpfree/actions/runs/42576743311) **passed all three jobs**: Rust tests and ARMv7 cross-build, shell/static no-write checks, and original C++ core/WAV tests.

The Rust job reported **5/5 M1 metronome tests and 6/6 new M2 groove tests passed in both debug and release**. The independent desktop WAV renderer printed:

~~~text
Rust MPFree groove: 176400 frames, 24 triggered events,
86436 prepared sample bytes, 0 stolen voices;
44,100 Hz stereo S32_LE WAV: /tmp/mpfree-ci-groove.wav
PASS: 2 bars, 24 drum events, 44.1kHz stereo S32_LE, nonzero audio
~~~

The same run preserved **zero sample difference** between Rust and the original C++ metronome at 120, 109, and 137.2 BPM. The Rust groove crate also compiled for ARMv7 ELF32 EABI5 hard-float. These are **real GitHub CI results**, not a claim that MPFree runs on the MPC.

The project's owner independently verified the M1 Rust metronome on **ML-tower**, using rustc/cargo **1.97.1**: 5/5 tests, all three C++/Rust parity comparisons at integer difference zero, creation of the 176,400-frame metronome WAV. **The new M2 drum demo has not yet been reported as played back on ML-tower**; instructions above reproduce it.

## What M2 does not implement

This is intentionally a *small, verifiable vertical slice*, not a full DAW. The following remain future work: user-supplied sample loading and streaming, actual velocity-layer instruments, zero-copy sample caching, MIDI capture, per-note sustain/off/release, swing, time signatures other than 4/4, tempo maps, parameter automation, polymeter, musical undo, per-track mixer/pan, effects routing, quality resampling, real plugin hosting, asynchronous app/UI state, project persistence, network audio transport, MPC hardware input/output, and crash recovery.

The synthetic sounds are only test assets; **factory instruments/samples are neither bundled nor distributed**. Future import must extract user-owned compatible content locally, as already described in [factory content strategy](../FACTORY_CONTENT.md).

## Next engineering gates

- **M3 transport:** separate sample-frame sequencer/event scheduling from the three-sound demo; support a typed note/automation event queue, track ID, tempo changes and per-track patterns without moving allocations into audio render.
- **M3 sampler:** read real user-provided WAVs at setup time (license-compliant), support velocity and note-off envelopes, bounded voice stealing with smooth fade, per-track gain and pan.
- **Network compute:** implement the [host-only framed audio model](AUXILIARY_COMPUTE_AND_IO.md) in Rust with exact protocol parity against the Python reference and bounded packet buffers.
- **Hardware compatibility:** validate target-image glibc and driver/runtime dependencies before attempting a *non-persistent* armv7-gnueabihf test on MPC. Do not stop `acvs.service` or acquire PCM/DRM/MIDI handles without a proven safe handoff and explicit agreement.
