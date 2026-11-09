# First actual Rust execution on the MPC One (without replacing Akai)

**Milestone:** Standalone Rust userspace proof running **alongside** the factory application, using the MPC's existing az0x 5.0.8 Linux and glibc 2.39. A later, independent MPFree application/userland is the priority. Building a minimal MPFree Linux distribution is **long-term optional**; replacing the Linux kernel is not planned.

**STATUS:** Host-only source, cross-build, runtime preflight and opt-in RAM-only launch implemented. **No assistant has run the command against the owner's MPC yet.** Factory service stop, restart, masking, rootfs/boot edits and exclusive ALSA/DRM/MIDI/input opens remain prohibited.

## Why this milestone matters

Observed MPC One firmware 3.6.0:

- Kernel `6.6.95-az01-2025-06-27-rt55`, ARMv7 Cortex-A17 family with NEON/VFP4.
- Native GNU libc release banner **2.39**, dynamic interpreter `/lib/ld-linux-armhf.so.3`. Target core libraries (ALSA, DRM, EGL, etc.) are present but not yet qualified for MPFree use.
- Stock `acvs.service` remains running and owns both ACVA PCM subdevices, MIDI, DRM and input. It holds ~1 GiB of resident/locked RAM. The system is ~2 GiB RAM with no swap.
- Factory Audio Processing and AudioWorker3 use CPU3 with SCHED_RR priority 20, while other workers occupy CPU0..2 at priority 20. A short **idle** CPU sample showed ~8% of whole-device capacity used by stock, not "two CPUs reserved by Linux."

A small non-hardware Rust executable is the **least intrusive first target proof**: it exercises architecture, glibc, ELF loading, Rust standard library, an actual MPFree audio computation, and memory/scheduling interactions. It does not prove real-time device performance or safe service handoff.

## Components

- `rust/mpfree-device-smoke`: no-unsafe Rust executable that runs the already-tested `mpfree-core::ClickEngine` for 4,096 frames at 44.1 kHz, verifies equal stereo, finite data, one beat trigger, nonzero samples and prints a deterministic checksum; **stdout only**, no filesystem, network, audio, graphics, MIDI or systemd API.
- `tools/mpc-one/host_ram_smoke.py`: host-side guarded driver. Reads a **live** on-device ABI survey first, confirms exact original MPC One DT model, az0x 5.0.8, kernel 6.6.95 series, embedded GNU libc 2.39. Uses the host ARM \`readelf\` checks to verify ELF32/EABI5 hard-float, interpreter, GNU libc version needs and every reported DT_NEEDED library name. Rejects absent evidence.
- Existing `tools/mpc-one/collect-over-ssh.sh`: same direct host interface, IP+MAC/ARP+route and **strict SSH known-host identity** gates as the previous successful read-only probes. New explicit modes:
  - `--ram-smoke-check`: **only read-only target ABI plus host ELF checks; no upload or execution**.
  - `--ram-smoke`: must also set `MPFREE_ACCEPT_VOLATILE_EXEC=YES`; **uploads & executes a small binary in tmpfs**, never persistent storage.
- Both modes store redaction-required host-side reports in ignored `research/`, with SHA-256 digests. The command does not accept arbitrary remote scripts or paths.

## Stage A: workstation prerequisites and native test

Owner has Rust 1.97.1 installed. From ML-tower:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main

# Native build and audio self-test (on ML-tower, not MPC):
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo run --manifest-path rust/Cargo.toml --release --locked \
  -p mpfree-device-smoke -- --self-test
~~~

Native Linux output should contain `MPFREE_ARMV7_SMOKE_V1`, `arch=x86_64`, 4,096 frames, one beat, `status=PASS`.

## Stage B: ARMv7 hard-float cross-build and ELF checks — ML-tower only

~~~bash
cd /home/jesse/code/mpfree-hardware-survey

rustup target add armv7-unknown-linux-gnueabihf

# Ubuntu/Kubuntu: GNU ARM hard-float linker and target libc development files.
sudo apt-get update
sudo apt-get install --no-install-recommends \
  gcc-arm-linux-gnueabihf binutils-arm-linux-gnueabihf libc6-dev-armhf-cross

CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-device-smoke

file rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-device-smoke
arm-linux-gnueabihf-readelf -h \
  rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-device-smoke
arm-linux-gnueabihf-readelf -l \
  rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-device-smoke
~~~

The target ELF must be ARM ELF32 EABI5 hard-float with PT_INTERP `/lib/ld-linux-armhf.so.3` and versioned-symbol requirements **no newer than glibc 2.39**. Cross-compiler installation changes the **workstation only**.

**Do not execute the ARM target executable on ML-tower** and do not copy it to MPC persistent directories (`/data`, `/etc`, `/var`, `/media`, `/boot`).

## Stage C: identity-guarded read-only compatibility check

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519

bash tools/mpc-one/collect-over-ssh.sh --ram-smoke-check
~~~

This first proves the device still matches the expected model/firmware, reads its actual libc release banner and library names, checks the candidate ELF (including `DT_NEEDED`), prints the local SHA-256 and **stops without uploading any bytes**.

If this fails, **do not bypass the check**. Share the redacted private report for a proper ABI/toolchain fix. A common legitimate blocker is an unobserved `libgcc_s.so.1` or a cross-built binary requiring newer glibc; the updated read-only ABI collector surveys additional runtime libraries. A different firmware/board or failing SSH host identity must also stop.

## Stage D: owner-approved, volatile first execution

**Only after Stage C passes**, save any stock project, stop recording/playback and confirm the MPC's factory UI is behaving normally.

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
MPFREE_ACCEPT_VOLATILE_EXEC=YES \
  bash tools/mpc-one/collect-over-ssh.sh --ram-smoke
~~~

The launcher rechecks the live ABI, requires `acvs.service` to be **active** with a valid MainPID, and verifies that `/run` is mounted **tmpfs**. It creates a unique root-owned mode-0700 temporary directory there, streams no more than **2 MiB** of locally built bytes into `probe`, and validates the remote SHA-256 before running the executable. It suppresses core dumps and regular-file writes for the child, uses a **five-second remote timeout** and `nice -n 10`, and deletes the temporary probe/directory via a remote exit trap.

Expected result (examples, checksum depends on architecture/libm):

~~~text
MPFREE_VOLATILE_STAGE_SHA256_OK
MPFREE_ARMV7_SMOKE_V1
os=linux
arch=arm
sample_rate_hz=44100
frames=4096
beats=1
nonzero_samples=...
checksum_fnv1a64=...
status=PASS
PASS: first on-target Rust sample computations; factory devices untouched.
~~~

**This is controlled volatile execution, not read-only:** it writes and removes a tiny file under RAM-backed `/run` and causes a short burst of CPU use. It does NOT stop/restart the factory application, acquire its PCM/DRM/MIDI/input devices, alter network settings, enable persistent services, remount/flash drives or access proprietary content.

If the SSH link drops before cleanup, an unused file may remain temporarily in `/run`; it disappears when tmpfs is cleared at reboot. The remote timeout prevents a surviving computation from running indefinitely. This is a *low-risk* smoke test, not a promise of zero possible runtime risk from running new code with the existing root-only access.

## Success criteria

1. Both stages meet existing IP/MAC/route + StrictHostKeyChecking/authorized SSH key requirements.
2. Live report confirms exact board+az0x build/kernel+libc release.
3. Candidate ELF passes native loader and GNU libc checks (fail-closed on unsupported dependencies).
4. Program runs once on ARM and exits in <5 seconds, with exact output markers and no disk/device handle interactions.
5. The factory `acvs.service` remains active before and after; owner independently checks touchscreen, pads, audio and GUI still work.
6. Temporary RAM-only executable disappears normally, and **no persistent filesystem modified**.

## What follows: a standalone MPFree application

This Rust smoke is the first *executable component* of the future MPFree **coexisting userspace**, not yet a user-facing DAW.

Next: create a `mpfree-daemon` or `mpfree-headless` executable with:
- a strictly bounded asynchronous control/event loop, off-thread project/sample loaders and external-compute client;
- pure-Rust sequencer + bounded mixer + sampler + network PCM model behind typed trait adapters;
- an audio HAL trait, with `NullAudioBackend` initially: validates rendered buffers and audio timing **without opening factory-owned ALSA**;
- bounded memory/CPU/latency metrics on actual ARMv7 (RSS/PSS), with zero allocations/locks in the render callback;
- future opt-in real ALSA/MIDI/DRM adapters, **disabled** until the independent factory recovery and watchdog handoff gates are satisfied.

Our primary objective is **replacing the factory application**, initially side by side, then through a reversible exclusive MPFree session. A custom MPC Linux distribution is a **longer-term option** once factory firmware restoration is independently proven; we are not replacing Linux itself.
