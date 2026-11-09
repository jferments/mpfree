# mpfree

**A proposed free, open-source music-production environment for the Akai MPC One.**

**Status: hardware discovery plus an initial desktop-testable independent audio core.** This repository does **not** yet contain a working MPC replacement application or firmware image. **On one MPC One running reported firmware 3.6.0, temporary root SSH access via USB autorun and a direct Ethernet connection was demonstrated on 2026-11-08.** The original firmware was not replaced.

## Vision

An offline-first, hardware-focused sampler, sequencer, arranger, synth/effects host, and production workstation that runs on the MPC One's Linux platform. Keep the tactile MPC workflow while introducing flexible clip and timeline arrangements, polymeter, parameter locks, conditional triggers, better automation, and a less menu-heavy UI.

## Starting point

We are beginning with a **userspace application**, not a replacement kernel, bootloader, or destructive firmware installer. We will first validate the hardware and build a reversible demonstration that can read a pad and output audio.

### Evidence-backed MPC One research

Start here for reproducible hardware/firmware development:

- [Verified MPC One 3.6.0 hardware/software baseline](docs/hardware/MPC_ONE_3_6_0_BASELINE.md) — observed ARMv7/PREEMPT_RT kernel, inMusic az0x distribution, storage mounts/overlays, and explicit unknowns.
- [Verified USB autorun and SSH developer access](docs/hardware/DEVELOPER_ACCESS.md) — upstream byte-sensitive autorun files, tested SSH approach, exact reference Ethernet lab settings, failure recovery.
- [Verified USB autorun reference builder and verifier](docs/hardware/USB_AUTORUN_REFERENCE.md) — exact scripts and upstream hashes from the successful reference device; **hardcoded lab-only USB safety checks, not a general installer**.
- [Hardware reverse-engineering research matrix](docs/hardware/RESEARCH_MATRIX.md) — subsystem-by-subsystem mapping plan with evidence labels and safety gates.
- [November 2026 in-device hardware survey](docs/hardware/MPC_ONE_3_6_0_SURVEY_20261109.md) — verified Rockchip RK3288 CPU/SoC, eMMC partition map, ACVA ALSA and USB-MIDI, DRM/Panfrost, ILI2117 touchscreen, process/service names, constraints and remaining questions.
- [Focused MPC process/device ownership survey](docs/hardware/MPC_ONE_3_6_0_RESOURCES_20261109.md) — actual `/proc/<acvs MainPID>/fd` links for PCM, MIDI, DRM, and evdev; 800x1280 DSI connector, Mali-T760-compatible GPU, 1.608 GHz clocks, USB identifiers and known BusyBox ELF probe limitation.
- [Active 44.1-kHz audio/ABI/JUCE/MIDI survey](docs/hardware/MPC_ONE_3_6_0_AUDIO_RUNTIME_20261109.md) — confirmed S32_LE stereo 128-frame PCM, independent playback/capture ring sizes, 38 stock app threads, high locked RAM, sequencer routing, EABI5 hard-float header.
- [First independent MPFree audio core (M0)](docs/engineering/FIRST_AUDIO_CORE.md) — C++17, deterministic metronome rendering and host-only stereo S32_LE WAV; tested without accessing MPC hardware.
- [Machine-readable hardware observations](docs/hardware/data/mpc-one-3.6.0-observed.json) — structured, **redacted** single-device evidence with explicit unknowns; no device serial or factory blobs.
- [One-USB installer architecture](docs/USB_INSTALLER_ARCHITECTURE.md) — future owner-consented installer, version compatibility, signatures, rollback and recovery.

A **read-only** automated survey, run on the *workstation*, is available as:

- [tools/mpc-one/collect-over-ssh.sh](tools/mpc-one/collect-over-ssh.sh) — checks interface/IP, network route, expected device MAC and SSH host key; writes report locally.
- [tools/mpc-one/read-only-inventory.sh](tools/mpc-one/read-only-inventory.sh) — POSIX shell survey streamed over SSH without installation or deliberate MPC writes.
- [tools/mpc-one/read-only-detail.sh](tools/mpc-one/read-only-detail.sh) — focused service/resource inventory, verified on target.
- [tools/mpc-one/read-only-runtime.sh](tools/mpc-one/read-only-runtime.sh) — targeted process threads and ALSA PCM procfs observation; **not yet exercised on MPC**.
- [tools/collect-hardware-info.sh](tools/collect-hardware-info.sh) — original lightweight device-local manual survey.

The SSH wrapper defaults to the **already-tested reference lab** (`eno1np0`, `10.42.0.141:2222`, and the existing dedicated developer key). Other users must set their own verified addresses, interface, MAC and key as described in the runbook.

Run this from a cloned copy of the branch containing these files (before merging architecture PR #1, that branch is `bootstrap/architecture-and-hardware-survey`):

~~~sh
bash tools/mpc-one/collect-over-ssh.sh

# Optional focused follow-up (no audio/MIDI opens or factory service changes):
# bash tools/mpc-one/collect-over-ssh.sh --detail
# bash tools/mpc-one/collect-over-ssh.sh --runtime
~~~

Raw survey output goes into local `research/`, which is ignored by Git. **Never commit unredacted hardware logs, private keys, factory firmware, sample archives or images.**

### First functional code: desktop-only MPFree audio engine

This repository now contains an **independent** sample-frame metronome core with no Akai firmware dependencies. Build and test on your workstation, **not on the MPC**:

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel 4
ctest --test-dir build --output-on-failure
./build/mpfree-render /tmp/mpfree-metronome.wav 4 120
file /tmp/mpfree-metronome.wav
~~~

The resulting WAV is **44,100 Hz / stereo / signed 32-bit little-endian PCM**, matching the observed ACVA stream format. A WAV written to ML-tower is not MPC playback; the ACVA device remains owned by the factory software. See [M0 implementation and safety gates](docs/engineering/FIRST_AUDIO_CORE.md).

### Architecture and project milestones

- [Architecture proposal](docs/ARCHITECTURE.md)
- [Roadmap and milestone gates](docs/ROADMAP.md)
- [Factory content and plugin compatibility policy](docs/FACTORY_CONTENT.md)

## Installation caution

There is **no MPFree end-user installation image yet**. Root SSH through the community autorun route is a research bootstrap specific to tested combinations and is not a universal firmware jailbreak. The MPC reference unit has a **read-only root but persistent writable overlays** on `/etc` and `/var`; careless edits can still persist or prevent normal startup. Do not repartition, flash, remount read-write or modify the factory boot chain during initial research.

## Legal and technical boundaries

mpfree will not distribute proprietary Akai/AIR firmware, plugin binaries, presets, or factory sample packs, or bypass activation. Optional compatibility with legitimately user-owned content can be researched independently. The original factory environment must remain recoverable.

## License

**Not selected yet.** We intend to use an OSI-approved free/open-source license, but do not assume that this repository is licensed for reuse until a license file is added.

## Research starting points

- [TheKikGen/MPC-LiveXplore](https://github.com/TheKikGen/MPC-LiveXplore)
- [TheKikGen/MPCLiveXplore-libs](https://github.com/TheKikGen/MPCLiveXplore-libs)

These are independent projects; review their licenses and hardware applicability before incorporating code.
