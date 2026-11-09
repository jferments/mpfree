# mpfree

**A proposed free, open-source music-production environment for the Akai MPC One.**

This repository does **not** yet contain a working MPC replacement application or firmware image, and the original firmware has not been replaced. Development happens on one MPC One running firmware 3.6.0, reached over a direct Ethernet link with root SSH (first via USB autorun on 2026-11-08, now persistent).

## Current status

| Milestone | What it proves | Where it ran |
|---|---|---|
| M0–M3 | Click, groove sequencer, WAV sampler and PCM network framing in Rust (C++ reference kept for parity) | Workstation and CI |
| M5 | Native ARMv7 Rust binary runs from RAM alongside the factory app | Physical MPC |
| M6 | Groove + sampler engine renders through a null audio backend (65× real time) | Physical MPC |
| M7 | 690 paced 128-frame callbacks, 0 synthetic deadline misses (p99 66 µs) | Physical MPC |
| M8 | 1280×800 logical test card and panel-rotation/touch transforms, offscreen | Workstation |
| M9–M10 | ILI2117 touch metadata: X/Y 0..=2048, 10 multitouch slots, read-only | Physical MPC |
| GPU | Surfaceless EGL/GLES 3.1 on Panfrost (Mali-T760), offscreen only | Physical MPC |
| M11 | Type-B multitouch decoder, pointer mapping and gesture model | Workstation and CI (synthetic events) |
| M12 | Read-only, step-by-step touch capture tool and orientation analyzer (implemented, owner run pending) | Workstation and CI; MPC run pending |

Not done yet: real ALSA playback, live touch events and their orientation, DRM display presentation, controller (pad/button/encoder) input, and a proven recovery path for the factory system. See the [roadmap](docs/ROADMAP.md) for gates and the [development log](docs/DEVELOPMENT_LOG.md) for the history.

## Vision

An offline-first, hardware-focused sampler, sequencer, arranger, synth/effects host, and production workstation that runs on the MPC One's Linux platform. Keep the tactile MPC workflow while introducing flexible clip and timeline arrangements, polymeter, parameter locks, conditional triggers, better automation, and a less menu-heavy UI.

## Starting point

We are beginning with a **userspace application**, not a replacement kernel, bootloader, or destructive firmware installer. Rust is the primary language ([ADR-0001](docs/engineering/ADR-0001-RUST-FIRST-HYBRID.md)), with narrow C/C++ interfaces where mature libraries are needed.

## Build and test

On a Linux workstation with Rust stable and CMake, from a checkout of `main`:

~~~sh
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo test --manifest-path rust/Cargo.toml --workspace --release --all-targets --locked

# C++ reference engine and parity check
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel 4
ctest --test-dir build --output-on-failure
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-render
python3 tests/check_cpp_rust_wav_parity.py --cpp build/mpfree-render --rust rust/target/release/mpfree-render
~~~

None of this touches the MPC. Commands that do are documented per milestone, and each one is opt-in and guarded; start with [developer access](docs/hardware/DEVELOPER_ACCESS.md).

## Documentation index

Hardware evidence and engineering notes, roughly in the order they were produced:

- [Verified MPC One 3.6.0 hardware/software baseline](docs/hardware/MPC_ONE_3_6_0_BASELINE.md) — observed ARMv7/PREEMPT_RT kernel, inMusic az0x distribution, storage mounts/overlays, and explicit unknowns.
- [Verified USB autorun and SSH developer access](docs/hardware/DEVELOPER_ACCESS.md) — upstream byte-sensitive autorun files, tested SSH approach, exact reference Ethernet lab settings, failure recovery.
- [Verified USB autorun reference builder and verifier](docs/hardware/USB_AUTORUN_REFERENCE.md) — exact scripts and upstream hashes from the successful reference device; **hardcoded lab-only USB safety checks, not a general installer**.
- [Hardware reverse-engineering research matrix](docs/hardware/RESEARCH_MATRIX.md) — subsystem-by-subsystem mapping plan with evidence labels and safety gates.
- [November 2026 in-device hardware survey](docs/hardware/MPC_ONE_3_6_0_SURVEY_20261109.md) — verified Rockchip RK3288 CPU/SoC, eMMC partition map, ACVA ALSA and USB-MIDI, DRM/Panfrost, ILI2117 touchscreen, process/service names, constraints and remaining questions.
- [Focused MPC process/device ownership survey](docs/hardware/MPC_ONE_3_6_0_RESOURCES_20261109.md) — actual `/proc/<acvs MainPID>/fd` links for PCM, MIDI, DRM, and evdev; 800x1280 DSI connector, Mali-T760-compatible GPU, 1.608 GHz clocks, USB identifiers and known BusyBox ELF probe limitation.
- [Active 44.1-kHz audio/ABI/JUCE/MIDI survey](docs/hardware/MPC_ONE_3_6_0_AUDIO_RUNTIME_20261109.md) — confirmed S32_LE stereo 128-frame PCM, independent playback/capture ring sizes, 38 stock app threads, high locked RAM, sequencer routing, EABI5 hard-float header.
- [First independent MPFree audio core (M0)](docs/engineering/FIRST_AUDIO_CORE.md) — original C++17 metronome and WAV proof of concept, retained for regression.
- [Rust M1 metronome parity and ARMv7 cross-build](docs/engineering/RUST_M1_PARITY.md) — **implemented** std-only Rust engine and WAV renderer, C++/Rust parity at 120/109/137.2 BPM, and verified ARM EABI5 hard-float cross-build; no MPC deployment.
- [Rust M2: 16-step sequencer and 16-voice groove sampler](docs/engineering/RUST_M2_GROOVE.md) — independent deterministic step scheduling, precompiled note events, synthetic kick/snare/hat one-shots, finite voice pool, offline WAV demo and regression tests; still no MPC hardware access.
- [Rust M3: bounded PCM packet codec and user-WAV sampler](docs/engineering/RUST_M3_NETWORK_SAMPLER.md) — Rust port of Python MPF1 protocol, jitter/deadline model, bounded 64-MiB total sample bank, velocity/note-off, offline preview and CI tests. No real sockets or MPC playback.
- [ARMv7 target userspace ABI survey and local comparator](docs/engineering/ARMV7_USERSPACE_COMPATIBILITY.md) — target libc 2.39 and matching EABI5/loader were owner-verified; M5/M6/M7 ARM application executions have passed, but broader dependencies and real drivers remain unqualified.
- [Reversible factory application handoff investigation](docs/engineering/REVERSIBLE_HANDOFF.md) — read-only systemd/resource preflight and independent recovery/watchdog prerequisite matrix; **does not stop factory app**.
- [Verified az0x ABI and factory handoff report (sanitized)](docs/hardware/MPC_ONE_3_6_0_ABI_HANDOFF_SURVEY_20261109.md) — real owner-run ABI/service data, CPU affinity and ~1 GiB locked process memory, with source SHA-256 fingerprints; raw device reports excluded.
- [Safe Rust handoff state-machine simulator](rust/mpfree-handoff-model/src/lib.rs) — guarded stock/MPFree transition model, mocked watchdog/crash/recovery failure tests; **no systemd, files, PCM or hardware calls**.
- [Hardware ownership strategy: independent MPFree Linux userspace first](docs/engineering/HARDWARE_OWNERSHIP_STRATEGY.md) — distinguishes factory application RSS from OS overhead and core affinity from real CPU utilization, with staged RAM-only tests, restore-before-stop gates and eventual optional custom rootfs.
- [M5: first RAM-only Rust executable on the MPC](docs/engineering/ARMV7_VOLATILE_SMOKE.md) — cross-built bounded Rust audio computation; live board/az0x 5.0.8/glibc 2.39/ELF checks; opt-in temporary `/run` execution with timeouts, SHA-256 validation and cleanup. Factory app remains active, with no PCM/MIDI/DRM acquisition.
- [M6: standalone headless groove + WAV sampler + null audio](docs/engineering/M6_HEADLESS_NULL_AUDIO.md) — **owner successfully ran the integrated engine on actual MPC One**: 88,200 frames in 30.565 ms (65.43× offline), 1,792 KiB RSS, same PCM checksum as workstation; factory app remained active and exclusive hardware untouched.
- [M7: synthetic 128-frame deadline timing + independent HOST supervisor](docs/engineering/M7_SYNTHETIC_TIMING_AND_HOST_SUPERVISOR.md) — physical MPC test **PASSED** under factory workload: 690 callbacks, 0 synthetic misses, p99 66µs, max 165µs, 167µs worst wakeup lateness, exact PCM parity (not ALSA).
- [M7 safe ARM acceptance](docs/engineering/M7_ARM_NULL_TIMING_ACCEPTANCE.md) — new read-only `--session-check` and independently allowlisted, double-opt-in `--session-smoke`, still null audio (no factory device handoff).
- [Factory process and reversible GUI handoff research](docs/engineering/FACTORY_GUI_HANDOFF_RESEARCH.md) — map running units, process/thread relationships and hardware fds without stopping Akai; independent recovery required before screen takeover.
- [ILI2117 touchscreen / Rockchip DSI display research](docs/hardware/MPC_ONE_TOUCHSCREEN_ILI2117_RESEARCH.md) — owner ran `--touchscreen` and `--process-map`; confirmed actual `ili2117` kernel binding, Linux Type-B multitouch bitmaps, independent controller and power-button services.
- [Sanitized physical M7/process/touchscreen acceptance evidence](docs/hardware/MPC_ONE_M7_PROCESS_TOUCH_VALIDATED_20261109.md) — owner reports interpreted with exact code-level observations, no raw private logs.
- [M8 host-only GUI test card, panel rotation and touch transform](docs/engineering/M8_DISPLAY_OFFSCREEN_TESTCARD.md) — new safe Rust `mpfree-display` draws 1280×800 logical card and 800×1280 clockwise/counterclockwise PPM previews **without DRM/evdev access**.
- [M9 Linux ILI2117 evdev ABS metadata interface](docs/engineering/M9_TOUCH_ABS_METADATA.md) — isolated Rust EVIOCGABS metadata-only bridge with strict driver binding and opt-in; **physical device execution subsequently verified in M10**.
- [M10 verified physical ILI2117 touch-axis metadata](docs/hardware/MPC_ONE_ILI2117_EVIOCGABS_VALIDATED_20261109.md) — owner-run double-opt-in read-only query passed on original MPC One: **X/Y=0..=2048, multitouch slots=10**, no events or grab; physical orientation remains unmeasured.
- [M10 guarded read-only ILI2117 launcher and acceptance runbook](docs/engineering/M10_GUARDED_TOUCH_AXES.md) — exact identity/ABI/sysfs and SHA preflight, RAM-only inspection and 5s watchdog; service-active is not proof of complete factory functionality.
- [M11 host-only multitouch decoder, pointer mapping and gestures](docs/engineering/M11_MULTITOUCH_DECODER.md) — Type-B slot/tracking-id decoding with SYN_REPORT frames and SYN_DROPPED resync, all eight swap/flip orientations, and tap/drag/long-press/two-finger recognition, tested against synthetic event streams only.
- [M12 live touch capture and orientation calibration](docs/engineering/M12_TOUCH_CAPTURE.md) — non-grabbing, read-only, RAM-only `event0` capture, one 9 s run per touch step after you press Enter, with eMMC write-counter checks, plus a host analyzer that recovers all 8 orientations from synthetic captures. **Owner run pending.**
- [Physical Panfrost/Mali-T760 EGL/GLES offscreen GPU qualification](docs/engineering/GPU_PANFROST_OFFSCREEN_20261109.md) — independently demonstrated surfaceless GLES 3.1 rendering on original MPC with Akai active (800×480 benchmark, p99 2.742ms); **not** real LCD/KMS presentation.
- [Accepted language architecture ADR: Rust-first with C/C++ compatibility](docs/engineering/ADR-0001-RUST-FIRST-HYBRID.md) — memory safety, ARMv7-gnueabihf, no-allocation real-time constraints, ALSA, Rust audio libraries, CLAP/VST3/LV2 gaps, Mali-T760 OpenGL ES graphics, migration/compatibility gates. **Accepted by the owner**; the C++ prototype is retained as a regression baseline.
- [Accepted audio backend ADR: direct ALSA on the MPC One](docs/engineering/ADR-0002-DIRECT-ALSA-AUDIO.md) — the engine owns the ACVA PCM; JACK/PipeWire only as desktop or optional backends.
- [Auxiliary computing over Ethernet and USB](docs/engineering/AUXILIARY_COMPUTE_AND_IO.md) — offload CPU-intensive synths/FX to another Linux computer, raw PCM streaming budget, latency/clock synchronization and deliberately separated passive I/O diagnosis versus optional bounded data transfer.
- [Measured MPC One Gigabit Ethernet and USB reference](docs/hardware/MPC_ONE_3_6_0_ETHERNET_USB_20261109.md) — **1000 Mbit/s full-duplex** Realtek RTL8211E, Rockchip GMAC, USB-A **480 Mbit/s**, DWC2 high-speed device controller, 0.272 ms average ping and bounded SSH goodput. Explicitly distinguishes raw wire speed, USB roles and unmeasured audio jitter.
- [Host-only network audio datagram prototype](tools/mpc-one/host_audio_packet.py) — sample-frame-stamped, MTU-checked S32_LE packet format and bounded deadline simulation, tested offline; not a live network receiver.
- [Machine-readable hardware observations](docs/hardware/data/mpc-one-3.6.0-observed.json) — structured, **redacted** single-device evidence with explicit unknowns; no device serial or factory blobs.
- [One-USB installer architecture](docs/USB_INSTALLER_ARCHITECTURE.md) — future owner-consented installer, version compatibility, signatures, rollback and recovery.

### Workstation survey tools

A **read-only** automated survey, run on the *workstation*, is available as:

- [tools/mpc-one/collect-over-ssh.sh](tools/mpc-one/collect-over-ssh.sh) — checks interface/IP, network route, expected device MAC and SSH host key; writes report locally.
- [tools/mpc-one/read-only-inventory.sh](tools/mpc-one/read-only-inventory.sh) — POSIX shell survey streamed over SSH without installation or deliberate MPC writes.
- [tools/mpc-one/read-only-detail.sh](tools/mpc-one/read-only-detail.sh) — focused service/resource inventory, verified on target.
- [tools/mpc-one/read-only-runtime.sh](tools/mpc-one/read-only-runtime.sh) — targeted process threads and ALSA PCM procfs observation; successfully exercised on MPC.
- [tools/mpc-one/read-only-io.sh](tools/mpc-one/read-only-io.sh) — **passive** sysfs and optional read-only ethtool report for Ethernet link/PHY and USB host/gadget capabilities.
- [tools/mpc-one/network-bench.py](tools/mpc-one/network-bench.py) — **explicit opt-in active** 20-ping and 4 MiB each direction SSH goodput check; workstation-hosted, writes no MPC files.
- [tools/collect-hardware-info.sh](tools/collect-hardware-info.sh) — original lightweight device-local manual survey.

The SSH wrapper defaults to the **already-tested reference lab** (`eno1np0`, `10.42.0.141:2222`, and the existing dedicated developer key). Other users must set their own verified addresses, interface, MAC and key as described in the runbook.

Run this from an up-to-date local checkout of `main`:

~~~sh
bash tools/mpc-one/collect-over-ssh.sh

# Optional focused follow-up (no audio/MIDI opens or factory service changes):
# bash tools/mpc-one/collect-over-ssh.sh --detail
# bash tools/mpc-one/collect-over-ssh.sh --runtime
# bash tools/mpc-one/collect-over-ssh.sh --io       # passive Ethernet/USB inventory
# bash tools/mpc-one/collect-over-ssh.sh --netbench # active 8 MiB total; only when idle
~~~

Raw survey output goes into local `research/`, which is ignored by Git. **Never commit unredacted hardware logs, private keys, factory firmware, sample archives or images.**

## Architecture and project milestones

- [Architecture proposal](docs/ARCHITECTURE.md)
- [Roadmap and milestone gates](docs/ROADMAP.md)
- [Development log](docs/DEVELOPMENT_LOG.md): the dated history that used to live in this README
- [Factory content and plugin compatibility policy](docs/FACTORY_CONTENT.md)

## Installation caution

There is **no MPFree end-user installation image yet**. Root SSH through the community autorun route is a research bootstrap specific to tested combinations and is not a universal firmware jailbreak. The MPC reference unit has a **read-only root but persistent writable overlays** on `/etc` and `/var`; careless edits can still persist or prevent normal startup. Do not repartition, flash, remount read-write or modify the factory boot chain during initial research.

## Legal and technical boundaries

mpfree will not distribute proprietary Akai/AIR firmware, plugin binaries, presets, or factory sample packs, or bypass activation. Optional compatibility with legitimately user-owned content can be researched independently. The original factory environment must remain recoverable.

## License

[MIT](LICENSE). This covers MPFree's own code and documentation only, not any Akai/AIR firmware, content or third-party projects referenced here.

## Research starting points

- [TheKikGen/MPC-LiveXplore](https://github.com/TheKikGen/MPC-LiveXplore)
- [TheKikGen/MPCLiveXplore-libs](https://github.com/TheKikGen/MPCLiveXplore-libs)

These are independent projects; review their licenses and hardware applicability before incorporating code.
