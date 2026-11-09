# MPFree M0: independent deterministic audio core (desktop-only)

**Status:** First real MPFree source code implemented under `src/core`; tested on GitHub-hosted x86-64 Ubuntu CI and suitable for local workstation use. This is **NOT** an MPC installation or a hardware driver. It does not modify the Akai device and cannot currently play through the ACVA ALSA card.

## Source provenance and requirements

Read-only real-device measurements:
- [Active audio and runtime survey](../hardware/MPC_ONE_3_6_0_AUDIO_RUNTIME_20261109.md), owner-local SHA-256 `1ea9eb27f837e5602cd5548db71622adf2f8c6aef52553f2aafd22e035a34b76`.
- Actual stock MPC audio parameters: `44100 Hz`, `S32_LE`, `2 channels`, interleaved, `128`-frame period, `384`-frame playback ALSA ring, `256`-frame capture ring.
- Stock application `acvs.service` owns both PCM streams and raw MIDI/DRM/evdev. Do **not** try to run MPFree concurrently on the ACVA hardware.
- Stock executable declares ARM ELF32 EABI5 hard-float; installed `/lib/ld-linux-armhf.so.3`; no ABI-compatible independent application has been deployed to MPC.

These settings inform only the **starting profile**. They are not an exhaustive capability list or a verified end-to-end latency measurement.

## Architecture

~~~text
src/core/include/mpfree/ClickEngine.hpp
    Config (44100, 120 BPM, 4 beats/bar)
    ClickEngine: sample-frame transport + click wavetable
    float_to_s32: saturation/PCM conversion value

src/core/ClickEngine.cpp
    configure() [non-RT]: validate, precompute sine/exp click samples
    render()    [RT]: no allocations or system I/O, deterministic
    reset()     [non-RT]: reset frame/beat positions

src/desktop/render_click_wav.cpp
    Host-only RIFF PCM WAV file writer, block_size=128
    Non-realtime, allowed to use filesystem I/O

tests/core_click_test.cpp
    Always-on checks under Release / NDEBUG
    Timing, block-size invariance, stereo, finite levels, saturation

CMakeLists.txt
    C++17 library mpfree-core; mpfree-render demo; CTest targets
~~~

### Rendering contract

`ClickEngine::render(left, right, frames)` writes two **planar** floating-point channels. The later ALSA backend will convert/interleave them to ACVA's observed stereo S32_LE stream. An actual backend will need verified handling of buffer mapping, ALSA interruption, recovery from xrun, and alignment; none exists yet.

The engine uses absolute sample-frame time. At 44.1 kHz / 120 BPM, clicks start at frames `0`, `22050`, `44100`, etc. For noninteger tempo, the next beat frame is calculated from the absolute beat index and rounded once: do **not** repeatedly round the period and add a fixed integer number of frames. This reduces systematic timing drift.

Every audio click's short waveform is generated into fixed arrays by `configure()`, outside the realtime render method. The render loop only compares frame indices, reads precomputed samples, writes floats and advances counters. There are no heap allocations, filesystem calls, logger calls, mutexes, or trigonometric functions in the callback path. Configuration and reset **must not run concurrently with rendering**.

Not yet implemented: MIDI clock, time signature changes mid-song, sample management, note scheduling beyond fixed metronome clicks, polyphonic voices, sample-rate conversion, effects, project persistence, realtime thread priority, UI, physical devices and AV synchronization.

### Float to signed 32-bit PCM

The `float_to_s32` utility deliberately clamps out-of-range values, handles non-finite values as silence and rounds floats within (-1,1) to signed 32-bit integers. Later use with ALSA must encode the resulting integer in **little-endian interleaved** order matching `S32_LE`. In a WAV file, interleaving and little-endian serialization are performed by the **desktop file writer**, not inside the realtime core.

This is a transport/conversion test, **not** evidence that the DAC has 32 effective audio bits.

## Build and run, exact current developer checkout

On **ML-tower**, not on the MPC:

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey

cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel 4
ctest --test-dir build --output-on-failure

./build/mpfree-render /tmp/mpfree-metronome.wav 4 120
file /tmp/mpfree-metronome.wav
~~~

The created `/tmp/mpfree-metronome.wav` is a 4-second, 44.1 kHz stereo, signed 32-bit little-endian PCM WAV generated entirely **on the workstation**. Use any local WAV player to hear it if desired; the file writer does not connect to the MPC and does not require ALSA devices.

### CI expectations

Existing `.github/workflows/mpc-inventory.yml` now builds the core on GitHub-hosted Ubuntu and executes:
- CMake + CTest deterministic render tests;
- a one-second offline WAV smoke test;
- Python validation of the WAV RIFF header, PCM format, 2 channels, 44,100 Hz, 32-bit sample width, byte rate, exact payload size and nonzero audio.

**CI status must be checked for the final PR commit.** A successful build of the desktop implementation is *not* validation of the MPC backend.

## Verified physical developer workstation acceptance (2026-11-08 Pacific / 2026-11-09 UTC)

The owner built the current `bootstrap/architecture-and-hardware-survey` branch **on ML-tower**, using GNU C++ **15.2.0** with a Release CMake build. This is a real developer-run check, independent of GitHub CI:

~~~text
[100%] Built target mpfree-core-test
[100%] Built target mpfree-render
1/2 Test #1: core-click-engine ........ Passed
2/2 Test #2: desktop-render-smoke ..... Passed
100% tests passed, 0 tests failed out of 2
Rendered 176400 frames; 8 beat triggers; 44,100 Hz stereo S32_LE WAV: /tmp/mpfree-metronome.wav
/tmp/mpfree-metronome.wav: RIFF (little-endian) data, WAVE audio,
  Microsoft PCM, 32 bit, stereo 44100 Hz
~~~

The user separately confirmed the four-second WAV **played back successfully** through their workstation's ordinary audio system. This validates build, offline PCM serialization and audible output on desktop, **not** realtime playback on MPC hardware or latency under load.

At 120 BPM there are 2 beat triggers per second, so 4 seconds contain **8 triggers**, with frames `0, 22050, 44100, ... 154350`. At 44.1 kHz, 4 seconds = `176400` frames. These are deterministic expected values, not subjective sound quality benchmarks.

Next milestone can proceed in parallel: expand host-only audio sequencing while investigating MPC Ethernet/USB offload potential via [auxiliary compute and interface survey](AUXILIARY_COMPUTE_AND_IO.md). Neither requires modifying the stock MPC services.

## Roadmap from M0

| Milestone | Output | Preconditions |
| --- | --- | --- |
| M0 | Standalone testable sample-frame click generator and offline S32 stereo WAV | **Implemented** |
| M1 | Hardware simulation input layer and deterministic note/transport event queue | Host-only unit tests |
| M2 | Portable bounded-memory sampler, sample reader and mixer with no allocations on audio path | Host-only tests and storage profiling |
| M3 | ARMv7 hard-float cross-build, ABI loader and library compatibility validation | Device sysroot/toolchain studies; **no factory binary redistributing** |
| M4 | Session-only ACVA playback after user consent and proven recovery/handoff | **BLOCKED:** no verified restore or stock service handoff |
| M5 | MPC public/private MIDI and evdev mapping via nonintrusive experiments | **BLOCKED:** event semantics unknown |
| M6 | DRM/Panfrost display and touchscreen presentation | **BLOCKED:** transform and DRM ownership tests |
| M7 | Reversible local USB app loader and fully automated installer | **BLOCKED:** proven fail-safe recovery and per-firmware compatibility required |

### Crucial safety boundary

`acvs.service` has `Restart=always`. A casual `kill` or `systemctl stop` is not a safe MPFree takeover mechanism. The factory app may hold the only available PCM subdevices and essential UI/control resources. Do not stop it or edit its unit file during initial research.

We can advance software features on the host without touching the MPC. The next hardware experiment should only read existing kernel counters and scheduler metadata; it should not require an exclusive PCM handle.
