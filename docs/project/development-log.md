# Development log

Dated narrative of how MPFree got to its current state, moved out of the
README on 2026-11-09. Each entry is kept as it was written at the time, so
"new" and "next" refer to that moment, not to today. Commands assume a local
checkout of `main`; substitute your own SSH key path and lab addresses (see
[developer-access](../guides/developer-access.md)).

For current status read the [README](../../README.md) and the
[roadmap](roadmap.md).

### First functional code: desktop-only MPFree audio engine

This repository now contains an **independent** sample-frame metronome core with no Akai firmware dependencies. Build and test on your workstation, **not on the MPC**:

~~~sh
cd mpfree  # your local checkout
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel 4
ctest --test-dir build --output-on-failure
./build/mpfree-render /tmp/mpfree-metronome.wav 4 120
file /tmp/mpfree-metronome.wav
~~~

The resulting WAV is **44,100 Hz / stereo / signed 32-bit little-endian PCM**, matching the observed ACVA stream format. A WAV written on the workstation is not MPC playback; the ACVA device remains owned by the factory software. See [M0 implementation and safety gates](../milestones/m00-cpp-audio-core.md).

## Ethernet and USB auxiliary-compute measurements (now confirmed)

The owner's 2026-11-09 `--io` and `--netbench` runs verified **1000-Mbit/s full-duplex Ethernet** on the actual MPC One (Rockchip `rk_gmac-dwmac`, Realtek RTL8211E PHY), **USB 2.0 high-speed 480 Mbit/s** to a removable storage device, and a registered `dwc2` high-speed **USB gadget UDC** which was *not attached* during the survey. Short ICMP ping averaged **0.272 ms** (20/20 replies); encrypted SSH goodput was **126.66 Mbit/s MPC→workstation** and **47.05 Mbit/s workstation→MPC** over only 4 MiB transfers, not a true UDP or gigabit PHY saturation test.

The [redacted measured reference](../hardware/ethernet-usb.md) preserves the hashes of the private raw logs and an evidence checklist. Raw PCM over gigabit Ethernet is technically plausible, but loss/jitter/clock drift/deadline tests and resource ownership remain blocking for live MPC audio output. For a first independent step, [host-only packet model](../../tools/mpc-one/host_audio_packet.py) tests MTU, timestamp, reorder, missed deadlines and finite buffering without sockets or hardware access.

## Accepted Rust-first architecture — first implementation now validated

**Rust-first with targeted C/C++ integration is now the accepted policy.** New MPFree-owned sequencing, realtime audio, networking and project logic should normally use Rust. Mature LV2/VST3 plugin hosts, synthesizers, DSP and low-level system libraries can be integrated through audited, narrow C/C++ interfaces. [ADR-0001](../decisions/adr-0001-rust-first.md) records the decision, and [M1](../milestones/m01-rust-metronome-parity.md) documents the first Rust audio prototype and ARM cross-build qualification.

The C++ reference is deliberately retained. Rust native tests and cross-language PCM WAV parity tests have **passed**, as has an ARMv7 hard-float cross-build on a CI workstation. **The M5–M7 ARMv7 binary and userspace null-audio runtime are proven on the physical MPC, but actual ACVA ALSA, DRM/evdev, full project performance, and app stop/restore remain unverified.**

On a Linux workstation (requires Rust stable and Cargo):

~~~sh
cd mpfree  # your local checkout
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo test --manifest-path rust/Cargo.toml --workspace --release --all-targets --locked
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-render
python3 tests/check_cpp_rust_wav_parity.py --cpp build/mpfree-render --rust rust/target/release/mpfree-render
./rust/target/release/mpfree-render /tmp/mpfree-rust-metronome.wav 4 120
xdg-open /tmp/mpfree-rust-metronome.wav
~~~

This only creates/plays a WAV on the Linux workstation. It does not touch the MPC.

## New Rust step sequencer and offline drum-machine demo (M2)

The `mpfree-groove` crate adds an actually playable 16-step drum sequence. A two-bar preset triggers kick, snare and hi-hat with sample-frame timestamps; waveforms and event lists are generated **outside** the audio render callback, which uses a fixed 16-voice pool. Sample data are synthetic and freely generated, not imported Akai content. On the workstation:

~~~sh
cd mpfree  # your local checkout
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-groove
./rust/target/release/mpfree-groove /tmp/mpfree-groove.wav 120 2
xdg-open /tmp/mpfree-groove.wav
~~~

Expected duration: 4 seconds, 176,400 frames, 24 scheduled drum events and stereo 44.1-kHz S32_LE PCM WAV. The precise [M2 architecture and safety limits](../milestones/m02-step-sequencer-sampler.md) are documented. This **does not** run or install anything on the MPC.

## M3: real WAV samples, Rust network framing and safe MPC preflight

The Rust workspace now includes `mpfree-sampler` (bounded, setup-time WAV PCM16/24/32/float32 decoding, note mapping, velocity, 16 voices and 64 MiB aggregate sample budget) and `mpfree-network` (Python-compatible 28-byte `MPF1` header, MTU checks, reordering/late-packet handling and bounded playout). **Neither opens actual MPC devices, and the network model does not open sockets.**

To preview your own 44.1-kHz WAV on the workstation:

~~~sh
cd mpfree  # your local checkout
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-sampler
./rust/target/release/mpfree-sampler /path/to/owned-sample.wav /tmp/mpfree-preview.wav
xdg-open /tmp/mpfree-preview.wav
~~~

To inspect the MPC's real az0x library ABI and service dependency/rollback requirements **without stopping anything** (same existing owner-verified Ethernet identity guard and temporary RAM SSH bootstrap):

~~~sh
ssh-add ~/.ssh/<your-mpc-developer-key>
bash tools/mpc-one/collect-over-ssh.sh --abi
bash tools/mpc-one/collect-over-ssh.sh --handoff-preflight
~~~

Attach the generated locally ignored `research/mpc-one-abi-*.txt` and `research/mpc-one-handoff-preflight-*.txt` for evaluation after checking for private identifiers. **Reversible handoff is research-only; independent recovery has NOT been proven.** See [M3 design](../milestones/m03-network-pcm-sampler.md).

## Current MPC ABI / reversible-handoff research (new read-only task survey)

The owner successfully ran the first `--abi` and `--handoff-preflight` surveys on the MPC One. The device has an ARMv7 hard-float loader, ALSA/DRM/EGL/GLES/GBM shared-library SONAMEs, and libc symbol strings up to `GLIBC_2.39`. **Exact GNU libc release is not established yet:** `getconf` was absent. The factory `acvs.service` owns the ACVA PCM devices and display, reports `Restart=always`, and locks about **1 GiB of RAM**. The main task reports CPU0 affinity but `isolcpus=2-3` means we must inspect per-task scheduling before designing our own realtime threads.

[Actual findings without identifiers](../hardware/abi-handoff-survey.md). A new Rust `mpfree-handoff-model` exercises failure and restore transitions **on a simulation only**, not the real MPC.

After updating `main`, run on the workstation:

~~~sh
cd mpfree  # your local checkout
git pull --ff-only origin main
cargo test --manifest-path rust/Cargo.toml --locked -p mpfree-handoff-model
ssh-add ~/.ssh/<your-mpc-developer-key>
bash tools/mpc-one/collect-over-ssh.sh --abi
bash tools/mpc-one/collect-over-ssh.sh --rtthreads
~~~

The SSH tools are passive/read-only and still enforce the exact lab Ethernet/ARP/device/host-key gates. Redact report identifiers before sharing. **Do not run a real service handoff yet; independent unbootable-device recovery is unproven.**

## Recovering the device's RAM and CPU without immediately replacing its OS

Owner-run reports identify a **~1,019 MiB factory application resident footprint** (roughly **1,016 MiB locked**) on a device with ~2,019 MiB Linux RAM. This is primarily **factory user-application memory, not an OS kernel reservation**. The factory application pins audio workers across CPUs 0–3 and the main `Audio Processing` thread to CPU3; `isolcpus=2-3` means scheduler isolation, **not** cores lost to Linux.

Our strategy is to **replace factory application/userspace first**, preserving the proven PREEMPT_RT kernel and ACVA/MIDI/DRM/input drivers until independent boot recovery and alternative hardware support are demonstrated. A true MPFree-owned Linux distribution remains a possible longer-term goal.

Read the [resource/accounting and OS ownership roadmap](../research/hardware-ownership.md). New passive `--capacity` reports actual CPU counter deltas, private/proportional memory when available, and pressure counters without stopping services, while updated `--rtthreads` now reports per-task scheduling policy/RT priority from kernel `stat`:

~~~sh
cd mpfree  # your local checkout
git pull --ff-only origin main
ssh-add ~/.ssh/<your-mpc-developer-key>
bash tools/mpc-one/collect-over-ssh.sh --capacity
python3 tools/mpc-one/analyze-capacity.py "$(ls -t research/mpc-one-capacity-*.txt | head -n 1)"
bash tools/mpc-one/collect-over-ssh.sh --rtthreads
~~~

These operations are research-only; **no factory services or device owners are changed**. Any stock-app handoff remains blocked until independent full factory recovery is proven.

## First ARMv7 Rust coexistence test (M5, opt-in)

The next proof of independent MPFree userspace is `mpfree-device-smoke`, a minimal Rust application that calculates 4,096 frames with the existing `ClickEngine`, reports a checksum, and exits. It does not open audio, MIDI, display, files or sockets.

An identity-guarded host runner provides a **read-only** live ARM ABI/ELF preflight, followed only on explicit owner opt-in by a **RAM-only** binary upload, SHA-256 verification, bounded timeout, and cleanup under `/run`; the factory application must remain active. This is *not* a factory service handoff or a custom OS installation.

See [full developer and safety commands](../milestones/m05-ram-only-rust-smoke.md). In particular, never run the execution mode until `--ram-smoke-check` passes and you've saved the current MPC project.
