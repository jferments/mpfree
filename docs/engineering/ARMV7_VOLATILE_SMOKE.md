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
  - `--ram-smoke-env`: **read-only prerequisite inventory** of utilities, `/run` tmpfs, POSIX CPU-limit support, normal scheduler policy, live stock service and ARM loader; does not require a compiled ELF.
  - `--ram-smoke-check`: **read-only ABI/ELF and remote utility checks; no upload or execution**.
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

## Observed M5 launch failure: status 127 (2026-11-09)

The owner successfully cross-compiled the Rust smoke ELF and passed the live read-only ARM ABI preflight. It established original MPC One / az0x 5.0.8 / GNU libc 2.39, correct EABI5 hard-float ELF interpreter and three candidate shared-library names, with maximum required GLIBC_2.34.

The **first opt-in launch failed with SSH remote exit 127 and empty stderr**, with no Rust status output. **A passed ABI check is not evidence that this executable ran.** Exit 127 may mean a required shell utility is absent, or the ELF interpreter/one of its dependencies failed at process start. The original launcher did not display the captured remote stdout on failure, so the failed stage is not yet known.

To isolate this WITHOUT another upload, a new read-only tool inspection was introduced. After merging this fix and updating ML-tower:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main

# NEW: read only the exact /run mount, stock service state and each utility.
bash tools/mpc-one/collect-over-ssh.sh --ram-smoke-env

# NEW: full original ABI check + read-only remote utility requirements.
bash tools/mpc-one/collect-over-ssh.sh --ram-smoke-check
~~~

The report prints `tool.timeout=present` or `tool.timeout=MISSING` and similarly for other commands, plus `run_mount_type` and `factory_service_active`. **A missing required tool prevents the upload** and yields an explicit message naming the tool. Do not install packages onto the MPC to work around this; we can instead implement and test a self-contained, bounded alternative when we know which components are unavailable.

For subsequent owner-authorized execution, the launcher now prints bounded `STAGE=` markers and the *last remote stdout and stderr* if it fails. This distinguishes failure before creating a temporary directory, during transfer/checksum, and when the Rust executable starts. **The follow-up report established the two missing commands: `timeout` and `nice`. Use the rebuilt watchdog-equipped binary and updated live preflight before any retry.**

These changes do not stop/restart the factory application, acquire exclusive devices, modify overlays, or bypass the existing owner opt-in, identity, ABI, tmpfs or execution-timeout safeguards.

## Resolution of the first exit-127 failure: missing GNU utilities

The owner ran the new **read-only** `--ram-smoke-env` on the actual original MPC One and observed:

| Requirement | MPC observation |
| --- | --- |
| `/run` filesystem | `tmpfs` (correct RAM-backed staging area) |
| `timeout` executable | **MISSING** |
| `nice` executable | **MISSING** |
| Other required utilities | `awk`, `mktemp`, `cat`, `sha256sum`, `chmod`, `rm`, `rmdir`, `systemctl` present |
| Factory `acvs.service` | Active with valid MainPID |
| ARM hard-float loader | Present |

Private on-workstation raw survey SHA-256: `ba013dc63e3f84db17f80f286e8bfae375e932c605ab3cb22c46590851c3ebe8`. No raw log or identifiers are committed.

This explains the earlier **remote shell exit 127 before binary launch**: the shell checked for executables `timeout` and `nice`, then failed before staging. It does **not** establish that the Rust executable ever loaded.

### New safe launcher without factory OS modifications

Do **not** install packages into az0x to work around missing tools. We now:

1. Treat `timeout` and `nice` as optional informational fields, not prerequisites. The factory SSH process is expected to inherit **SCHED_OTHER (policy 0)**, lower priority than the factory's SCHED_RR audio threads; the read-only survey checks the numeric policy.
2. Verify a POSIX `ulimit -t 3` limit is supported **in a disposable child shell**, without altering factory thread resource limits.
3. Enforce `ulimit -t 3` in the actual remote shell before execution, providing a **kernel CPU-time limit** in addition to core/file size limits.
4. Arm a safe-Rust **five-second wall-clock watchdog thread** inside the Rust smoke binary, which aborts the process if the one-shot computation does not complete. This does not require `unsafe` or external utilities.
5. Retain the host's **18-second SSH execution deadline**, exact board identity, glibc/ELF and library checks, SHA-256 before execute, live factory-app state, explicit opt-in, tmpfs-only staging, and removal trap.

**Safety limitation:** The Rust watchdog is **inside the test process**, not a separate privileged supervisor. An unschedulable/uninterruptible process or a killed SSH session can defeat some userspace guarantees. A POSIX CPU-time limit measures *actual execution time*, not wall-clock time. This bounded smoke is therefore **not** a validated general-purpose safe application handoff. We will implement a separate supervisor before any persistent DAW or exclusive resource ownership.

### Workstation and read-only checks before reattempting execution

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main

# IMPORTANT: rebuild cross-target ELF; the previously built candidate
# does NOT contain the mandatory Rust watchdog and is rejected by the host.
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-device-smoke

cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
bash tools/mpc-one/collect-over-ssh.sh --ram-smoke-check
~~~

The live preflight must print **`PASS: ... 8 required utilities ... shell CPU limit, SCHED_OTHER; timeout/nice not required`** before any opt-in execution. If the shell policy or CPU limit is unavailable, **stop** rather than bypassing it.

## Stage D: owner-approved, volatile first execution

**Only after Stage C passes**, save any stock project, stop recording/playback and confirm the MPC's factory UI is behaving normally.

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
MPFREE_ACCEPT_VOLATILE_EXEC=YES \
  bash tools/mpc-one/collect-over-ssh.sh --ram-smoke
~~~

The launcher rechecks the live ABI, requires `acvs.service` to be **active** with a valid MainPID, confirms inherited **SCHED_OTHER** and POSIX CPU-limit support, and verifies that `/run` is **tmpfs**. It creates a unique root-owned mode-0700 temporary directory there, streams no more than **2 MiB** into `probe`, and validates the remote SHA-256 before running the executable. It suppresses core dumps and regular-file writes, applies `ulimit -t 3` and uses the **Rust program's own five-second watchdog thread** plus a host SSH deadline. It removes the temporary files using an exit trap. Neither `timeout` nor `nice` is required on the MPC.

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

If the SSH link drops before cleanup, an unused file may remain temporarily in `/run`; it disappears when tmpfs is cleared at reboot. The Rust-internal watchdog and Linux CPU-time limit bound ordinary computation failures, although they are not equivalent to a separate independent watchdog supervisor. This is a *low-risk* smoke test, not a promise of zero possible runtime risk from running new code with the existing root-only access.

**Persistence nuance:** Our installer/launcher intentionally writes no files outside `/run`, but stock `sshd`, PAM or system logging may independently append ordinary login/audit entries to writable overlays during a session. This is not a forensic zero-change experiment. It is a **reversible application-space test** that does not alter firmware, boot settings, installed programs, persistent service configuration, audio device ownership or user projects. Use the vendor UI to confirm nothing unexpected changed after the test.

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
