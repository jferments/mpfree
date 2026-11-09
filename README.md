# mpfree

**A proposed free, open-source music-production environment for the Akai MPC One.**

**Status: hardware discovery plus desktop-testable Rust metronome, step sequencer, bounded WAV sampler and network-audio packet prototypes.** This repository does **not** yet contain a working MPC replacement application or firmware image. **On one MPC One running reported firmware 3.6.0, temporary root SSH access via USB autorun and a direct Ethernet connection was demonstrated on 2026-11-08.** The original firmware was not replaced.

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
- [First independent MPFree audio core (M0)](docs/engineering/FIRST_AUDIO_CORE.md) — original C++17 metronome and WAV proof of concept, retained for regression.
- [Rust M1 metronome parity and ARMv7 cross-build](docs/engineering/RUST_M1_PARITY.md) — **implemented** std-only Rust engine and WAV renderer, C++/Rust parity at 120/109/137.2 BPM, and verified ARM EABI5 hard-float cross-build; no MPC deployment.
- [Rust M2: 16-step sequencer and 16-voice groove sampler](docs/engineering/RUST_M2_GROOVE.md) — independent deterministic step scheduling, precompiled note events, synthetic kick/snare/hat one-shots, finite voice pool, offline WAV demo and regression tests; still no MPC hardware access.
- [Rust M3: bounded PCM packet codec and user-WAV sampler](docs/engineering/RUST_M3_NETWORK_SAMPLER.md) — Rust port of Python MPF1 protocol, jitter/deadline model, bounded 64-MiB total sample bank, velocity/note-off, offline preview and CI tests. No real sockets or MPC playback.
- [ARMv7 target userspace ABI survey and local comparator](docs/engineering/ARMV7_USERSPACE_COMPATIBILITY.md) — read-only libc/loader/library probe and host-only ELF/glibc requirement checker; target compatibility remains unverified.
- [Reversible factory application handoff investigation](docs/engineering/REVERSIBLE_HANDOFF.md) — read-only systemd/resource preflight and independent recovery/watchdog prerequisite matrix; **does not stop factory app**.
- [Verified az0x ABI and factory handoff report (sanitized)](docs/hardware/MPC_ONE_3_6_0_ABI_HANDOFF_SURVEY_20261109.md) — real owner-run ABI/service data, CPU affinity and ~1 GiB locked process memory, with source SHA-256 fingerprints; raw device reports excluded.
- [Safe Rust handoff state-machine simulator](rust/mpfree-handoff-model/src/lib.rs) — guarded stock/MPFree transition model, mocked watchdog/crash/recovery failure tests; **no systemd, files, PCM or hardware calls**.
- [Hardware ownership strategy: independent MPFree Linux userspace first](docs/engineering/HARDWARE_OWNERSHIP_STRATEGY.md) — distinguishes factory application RSS from OS overhead and core affinity from real CPU utilization, with staged RAM-only tests, restore-before-stop gates and eventual optional custom rootfs.
- [M5: first RAM-only Rust executable on the MPC](docs/engineering/ARMV7_VOLATILE_SMOKE.md) — cross-built bounded Rust audio computation; live board/az0x 5.0.8/glibc 2.39/ELF checks; opt-in temporary `/run` execution with timeouts, SHA-256 validation and cleanup. Factory app remains active, with no PCM/MIDI/DRM acquisition.
- [M6: standalone headless groove + WAV sampler + null audio](docs/engineering/M6_HEADLESS_NULL_AUDIO.md) — owner M5 ARMv7 execution is proven; M6 integrates real engines behind a `NullAudioBackend`. New `--headless-check` and dual-opt-in `--headless-smoke` permit a finite, guarded RAM-only on-device test; **M6 hardware execution not yet demonstrated** and no factory device endpoints are opened.
- [Accepted language architecture ADR: Rust-first with C/C++ compatibility](docs/engineering/ADR-0001-RUST-FIRST-HYBRID.md) — memory safety, ARMv7-gnueabihf, no-allocation real-time constraints, ALSA, Rust audio libraries, CLAP/VST3/LV2 gaps, Mali-T760 OpenGL ES graphics, migration/compatibility gates. **Accepted by the owner**; the C++ prototype is retained as a regression baseline.
- [Auxiliary computing over Ethernet and USB](docs/engineering/AUXILIARY_COMPUTE_AND_IO.md) — offload CPU-intensive synths/FX to another Linux computer, raw PCM streaming budget, latency/clock synchronization and deliberately separated passive I/O diagnosis versus optional bounded data transfer.
- [Measured MPC One Gigabit Ethernet and USB reference](docs/hardware/MPC_ONE_3_6_0_ETHERNET_USB_20261109.md) — **1000 Mbit/s full-duplex** Realtek RTL8211E, Rockchip GMAC, USB-A **480 Mbit/s**, DWC2 high-speed device controller, 0.272 ms average ping and bounded SSH goodput. Explicitly distinguishes raw wire speed, USB roles and unmeasured audio jitter.
- [Host-only network audio datagram prototype](tools/mpc-one/host_audio_packet.py) — sample-frame-stamped, MTU-checked S32_LE packet format and bounded deadline simulation, tested offline; not a live network receiver.
- [Machine-readable hardware observations](docs/hardware/data/mpc-one-3.6.0-observed.json) — structured, **redacted** single-device evidence with explicit unknowns; no device serial or factory blobs.
- [One-USB installer architecture](docs/USB_INSTALLER_ARCHITECTURE.md) — future owner-consented installer, version compatibility, signatures, rollback and recovery.

A **read-only** automated survey, run on the *workstation*, is available as:

- [tools/mpc-one/collect-over-ssh.sh](tools/mpc-one/collect-over-ssh.sh) — checks interface/IP, network route, expected device MAC and SSH host key; writes report locally.
- [tools/mpc-one/read-only-inventory.sh](tools/mpc-one/read-only-inventory.sh) — POSIX shell survey streamed over SSH without installation or deliberate MPC writes.
- [tools/mpc-one/read-only-detail.sh](tools/mpc-one/read-only-detail.sh) — focused service/resource inventory, verified on target.
- [tools/mpc-one/read-only-runtime.sh](tools/mpc-one/read-only-runtime.sh) — targeted process threads and ALSA PCM procfs observation; successfully exercised on MPC.
- [tools/mpc-one/read-only-io.sh](tools/mpc-one/read-only-io.sh) — **passive** sysfs and optional read-only ethtool report for Ethernet link/PHY and USB host/gadget capabilities.
- [tools/mpc-one/network-bench.py](tools/mpc-one/network-bench.py) — **explicit opt-in active** 20-ping and 4 MiB each direction SSH goodput check; workstation-hosted, writes no MPC files.
- [tools/collect-hardware-info.sh](tools/collect-hardware-info.sh) — original lightweight device-local manual survey.

The SSH wrapper defaults to the **already-tested reference lab** (`eno1np0`, `10.42.0.141:2222`, and the existing dedicated developer key). Other users must set their own verified addresses, interface, MAC and key as described in the runbook.

Run this from a cloned copy of the branch containing these files (before merging architecture PR #1, that branch is `bootstrap/architecture-and-hardware-survey`):

~~~sh
bash tools/mpc-one/collect-over-ssh.sh

# Optional focused follow-up (no audio/MIDI opens or factory service changes):
# bash tools/mpc-one/collect-over-ssh.sh --detail
# bash tools/mpc-one/collect-over-ssh.sh --runtime
# bash tools/mpc-one/collect-over-ssh.sh --io       # passive Ethernet/USB inventory
# bash tools/mpc-one/collect-over-ssh.sh --netbench # active 8 MiB total; only when idle
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

### Ethernet and USB auxiliary-compute measurements (now confirmed)

The owner's 2026-11-09 §--io§ and §--netbench§ runs verified **1000-Mbit/s full-duplex Ethernet** on the actual MPC One (Rockchip §rk_gmac-dwmac§, Realtek RTL8211E PHY), **USB 2.0 high-speed 480 Mbit/s** to a removable storage device, and a registered §dwc2§ high-speed **USB gadget UDC** which was *not attached* during the survey. Short ICMP ping averaged **0.272 ms** (20/20 replies); encrypted SSH goodput was **126.66 Mbit/s MPC→workstation** and **47.05 Mbit/s workstation→MPC** over only 4 MiB transfers, not a true UDP or gigabit PHY saturation test.

The [redacted measured reference](docs/hardware/MPC_ONE_3_6_0_ETHERNET_USB_20261109.md) preserves the hashes of the private raw logs and an evidence checklist. Raw PCM over gigabit Ethernet is technically plausible, but loss/jitter/clock drift/deadline tests and resource ownership remain blocking for live MPC audio output. For a first independent step, [host-only packet model](tools/mpc-one/host_audio_packet.py) tests MTU, timestamp, reorder, missed deadlines and finite buffering without sockets or hardware access.

### Accepted Rust-first architecture — first implementation now validated

**Rust-first with targeted C/C++ integration is now the accepted policy.** New MPFree-owned sequencing, realtime audio, networking and project logic should normally use Rust. Mature LV2/VST3 plugin hosts, synthesizers, DSP and low-level system libraries can be integrated through audited, narrow C/C++ interfaces. [ADR-0001](docs/engineering/ADR-0001-RUST-FIRST-HYBRID.md) records the decision, and [M1](docs/engineering/RUST_M1_PARITY.md) documents the first Rust audio prototype and ARM cross-build qualification.

The C++ reference is deliberately retained. Rust native tests and cross-language PCM WAV parity tests have **passed**, as has an ARMv7 hard-float cross-build on a CI workstation. **Binary compatibility with the MPC's az0x userspace and on-device execution remain unverified.**

On ML-tower (requires Rust stable and Cargo):

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo test --manifest-path rust/Cargo.toml --workspace --release --all-targets --locked
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-render
python3 tests/check_cpp_rust_wav_parity.py --cpp build/mpfree-render --rust rust/target/release/mpfree-render
./rust/target/release/mpfree-render /tmp/mpfree-rust-metronome.wav 4 120
xdg-open /tmp/mpfree-rust-metronome.wav
~~~

This only creates/plays a WAV on the Linux workstation. It does not touch the MPC.

### New Rust step sequencer and offline drum-machine demo (M2)

The `mpfree-groove` crate adds an actually playable 16-step drum sequence. A two-bar preset triggers kick, snare and hi-hat with sample-frame timestamps; waveforms and event lists are generated **outside** the audio render callback, which uses a fixed 16-voice pool. Sample data are synthetic and freely generated, not imported Akai content. On ML-tower:

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-groove
./rust/target/release/mpfree-groove /tmp/mpfree-groove.wav 120 2
xdg-open /tmp/mpfree-groove.wav
~~~

Expected duration: 4 seconds, 176,400 frames, 24 scheduled drum events and stereo 44.1-kHz S32_LE PCM WAV. The precise [M2 architecture and safety limits](docs/engineering/RUST_M2_GROOVE.md) are documented. This **does not** run or install anything on the MPC.

### M3: real WAV samples, Rust network framing and safe MPC preflight

The Rust workspace now includes `mpfree-sampler` (bounded, setup-time WAV PCM16/24/32/float32 decoding, note mapping, velocity, 16 voices and 64 MiB aggregate sample budget) and `mpfree-network` (Python-compatible 28-byte `MPF1` header, MTU checks, reordering/late-packet handling and bounded playout). **Neither opens actual MPC devices, and the network model does not open sockets.**

To preview your own 44.1-kHz WAV on ML-tower:

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-sampler
./rust/target/release/mpfree-sampler /path/to/owned-sample.wav /tmp/mpfree-preview.wav
xdg-open /tmp/mpfree-preview.wav
~~~

To inspect the MPC's real az0x library ABI and service dependency/rollback requirements **without stopping anything** (same existing owner-verified Ethernet identity guard and temporary RAM SSH bootstrap):

~~~sh
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash tools/mpc-one/collect-over-ssh.sh --abi
bash tools/mpc-one/collect-over-ssh.sh --handoff-preflight
~~~

Attach the generated locally ignored `research/mpc-one-abi-*.txt` and `research/mpc-one-handoff-preflight-*.txt` for evaluation after checking for private identifiers. **Reversible handoff is research-only; independent recovery has NOT been proven.** See [M3 design](docs/engineering/RUST_M3_NETWORK_SAMPLER.md).

### Current MPC ABI / reversible-handoff research (new read-only task survey)

The owner successfully ran the first `--abi` and `--handoff-preflight` surveys on the MPC One. The device has an ARMv7 hard-float loader, ALSA/DRM/EGL/GLES/GBM shared-library SONAMEs, and libc symbol strings up to `GLIBC_2.39`. **Exact GNU libc release is not established yet:** `getconf` was absent. The factory `acvs.service` owns the ACVA PCM devices and display, reports `Restart=always`, and locks about **1 GiB of RAM**. The main task reports CPU0 affinity but `isolcpus=2-3` means we must inspect per-task scheduling before designing our own realtime threads.

[Actual findings without identifiers](docs/hardware/MPC_ONE_3_6_0_ABI_HANDOFF_SURVEY_20261109.md). A new Rust `mpfree-handoff-model` exercises failure and restore transitions **on a simulation only**, not the real MPC.

After updating `main`, run on ML-tower:

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
cargo test --manifest-path rust/Cargo.toml --locked -p mpfree-handoff-model
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash tools/mpc-one/collect-over-ssh.sh --abi
bash tools/mpc-one/collect-over-ssh.sh --rtthreads
~~~

The SSH tools are passive/read-only and still enforce the exact lab Ethernet/ARP/device/host-key gates. Redact report identifiers before sharing. **Do not run a real service handoff yet; independent unbootable-device recovery is unproven.**

### Recovering the device's RAM and CPU without immediately replacing its OS

Owner-run reports identify a **~1,019 MiB factory application resident footprint** (roughly **1,016 MiB locked**) on a device with ~2,019 MiB Linux RAM. This is primarily **factory user-application memory, not an OS kernel reservation**. The factory application pins audio workers across CPUs 0–3 and the main `Audio Processing` thread to CPU3; `isolcpus=2-3` means scheduler isolation, **not** cores lost to Linux.

Our strategy is to **replace factory application/userspace first**, preserving the proven PREEMPT_RT kernel and ACVA/MIDI/DRM/input drivers until independent boot recovery and alternative hardware support are demonstrated. A true MPFree-owned Linux distribution remains a possible longer-term goal.

Read the [resource/accounting and OS ownership roadmap](docs/engineering/HARDWARE_OWNERSHIP_STRATEGY.md). New passive `--capacity` reports actual CPU counter deltas, private/proportional memory when available, and pressure counters without stopping services, while updated `--rtthreads` now reports per-task scheduling policy/RT priority from kernel `stat`:

~~~sh
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash tools/mpc-one/collect-over-ssh.sh --capacity
python3 tools/mpc-one/analyze-capacity.py "$(ls -t research/mpc-one-capacity-*.txt | head -n 1)"
bash tools/mpc-one/collect-over-ssh.sh --rtthreads
~~~

These operations are research-only; **no factory services or device owners are changed**. Any stock-app handoff remains blocked until independent full factory recovery is proven.

### First ARMv7 Rust coexistence test (M5, opt-in)

The next proof of independent MPFree userspace is `mpfree-device-smoke`, a minimal Rust application that calculates 4,096 frames with the existing `ClickEngine`, reports a checksum, and exits. It does not open audio, MIDI, display, files or sockets.

An identity-guarded host runner provides a **read-only** live ARM ABI/ELF preflight, followed only on explicit owner opt-in by a **RAM-only** binary upload, SHA-256 verification, bounded timeout, and cleanup under `/run`; the factory application must remain active. This is *not* a factory service handoff or a custom OS installation.

See [full developer and safety commands](docs/engineering/ARMV7_VOLATILE_SMOKE.md). In particular, never run the execution mode until `--ram-smoke-check` passes and you've saved the current MPC project.

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
