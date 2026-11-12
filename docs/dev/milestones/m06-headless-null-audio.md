# M6: independent headless MPFree engine with null audio

**Status:** M6 native tests, ARMv7 cross-build and **physical MPC One execution all passed** (owner-run 2026-11-09). The double-opt-in RAM-only `--headless-smoke` returned `status=PASS`, 88,200 frames, 12 groove events, 4 sampled notes, 30,565 µs offline render time (**65.43×**), 1,792 KiB current/peak RSS and the previously verified x86-64 PCM checksum. The factory application remained active in the launcher checks and retains audio/MIDI/display/input ownership. M6 remains a finite null-output test, **not a persistent daemon, real-time driver, or factory handoff**. See [M7 timing architecture](m07-synthetic-timing.md).

## M5 completed on original MPC One — actual owner evidence

The first ARMv7 smoke test ran from a unique temporary directory under verified /run tmpfs using guarded Ethernet/SSH. The original Akai app remained active, as verified before and after execution.

- **Live target:** original Akai MPC One, az0x 5.0.8, glibc 2.39, ARM ELF32 EABI5 hard-float.
- **Executable:** 531,960 bytes, max GLIBC requirement 2.34, libraries libc.so.6/libgcc_s.so.1/libm.so.6.
- **Local binary SHA-256:** `65f2c2a10a6421b40561b0d0c868be600290a10ae8fb4e616bb0ca2e803bd1fe`
- **Host-side private report SHA-256:** `776de1db00440dc69e34260ecef9753ec97559dd1e742545a04477eaadf9a07d`.
- **On-target result:** `MPFREE_ARMV7_SMOKE_V1`, `os=linux`, `arch=arm`, `sample_rate_hz=44100`, `frames=4096`, `beats=1`, `nonzero_samples=153`, `checksum_fnv1a64=295b5aeef41825c6`, `status=PASS`.
- **Safety evidence:** `MPFREE_VOLATILE_STAGE_SHA256_OK`, `STAGE=RUST_EXECUTED`, `WATCHDOG=RUST_5S_ARMED`; launcher reported removal of the temporary file. Verification was done with factory app active. **This proves a short-lived application-space coexistence computation, not physical audio playback or worst-case real-time safety.**
- The full raw reports remain only under the owner's ignored `research/` folder. No device MAC, serial numbers, or raw session logs are committed to GitHub.

## M6 architecture

~~~text
              MPFree headless Rust binary
                        |
          finite CLI self-test + 5s watchdog
                        |
                HeadlessSession
               /       |       \
        GrooveEngine  Sampler    audio sink trait
         16 steps      WAV PCM   (AudioSink::write)
               \       /               |
                per-128-frame mix --> NullAudioBackend
                                        |
                     PCM checks, peak, frames, FNV checksum
                                        |
                                stdout metrics only
~~~

- A **real** `GrooveEngine` runs the default 16-step synthesized kick/snare/hats pattern, 120 BPM, 44.1 kHz for one bar (2.0 seconds of *audio frames*).
- The existing **WAV parser** and `Sampler` render a newly generated **synthetic PCM16 sample**, prepared once **outside render** and mapped to MIDI note 60. A note is triggered at each exact 22,050-frame interval (0, 0.5, 1.0, 1.5 seconds). MIDI/input hardware is never accessed.
- `HeadlessSession::render` handles variable callback sizes and splits blocks at sample-exact note timestamps; all audio buffers are allocated before entering the render path.
- `NullAudioBackend` accepts stereo float slices, validates finite values, counts frames/nonzero samples, tracks peak amplitude and hashes quantized stereo S32_LE samples. **It does not open any audio device, write a WAV, network socket or file, or acquire physical controller/display resources**.
- The executable's five-second Rust watchdog is for this **finite computation only**. There is no persistent daemon, async event loop, remote control API, production watchdog or real-time scheduler in M6. CPU performance measured for this offline render must not be interpreted as proven xrun-free hardware playback.
- The executable also reads **only `/proc/self/status`, after finishing the DSP work**, to report `peak_rss_kib` (process high-water resident set) and `rss_kib` (current resident set). They may be `unavailable` on kernels that omit them. These are process memory estimates, **not** exclusive PSS or a complete OS memory budget. The offline `elapsed_us` metric permits calculation of processing speed but does not prove individual 128-frame ALSA callback deadlines.

## Host build/test (no MPC involved)

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo run --manifest-path rust/Cargo.toml --release --locked \
  -p mpfree-headless -- --self-test
~~~

Expected key fields:

~~~text
MPFREE_HEADLESS_V1
os=linux
arch=x86_64
backend=null
sample_rate_hz=44100
frames=88200
groove_events=12
sampler_notes=4
groove_voice_steals=0
sampler_voice_steals=0
...
status=PASS
~~~

The process emits `MPFREE_HEADLESS_RUST_WATCHDOG_5S_ARMED` on stderr. The exact checksum and performance time are measured, not predetermined. Unit tests include variable-block partition equivalence, invalid buffer lengths, nonfinite PCM rejection and event counts.

## ARM cross-build (host-only)

~~~bash
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-headless

file rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-headless
~~~

CI validates ARM EABI5 hard-float ELF32, `/lib/ld-linux-armhf.so.3` and glibc requirements ≤2.39. **Cross-building M6 does not mean it has executed on the physical MPC**.

## M6 physical MPC acceptance: allowlisted RAM-only test (OWNER-RUN)

The existing M5 `--ram-smoke` is **unchanged**, and remains pinned to `mpfree-device-smoke`. New modes explicitly pin `mpfree-headless` to a separate, preapproved `headless` profile with its own output version, watchdog marker, exact note/event/frame counts and **`backend=null`** requirement. Supplying a different executable name or an arbitrary remote shell script is not accepted.

This runner reuses the successful M5 protections: exact direct Ethernet+MAC+SSH key/host identity; live original MPC One / az0x 5.0.8 / glibc 2.39 ABI; ARMv7 ELF32/EABI5 hard float interpreter and linked libraries; verified `/run` tmpfs; active `acvs.service` and live MainPID; inherited SCHED_OTHER normal policy; `ulimit -t 3` CPU-time / `ulimit -c 0` core / `ulimit -f 0` regular-file guard; SHA-256 of streamed ELF before execution; 5-second **in-process** Rust watchdog; a bounded host SSH deadline; and cleanup using a shell exit trap. Nothing accesses ALSA, MIDI, DRM or hardware input nodes.

### 1. Workstation update, build and tests

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main

cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked

CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-headless
~~~

**Rebuilding is required:** the ARM binary should now include the new post-render process-RSS reporting. The host-side launcher also rejects binaries missing the exact `MPFREE_HEADLESS_RUST_WATCHDOG_5S_ARMED` and `MPFREE_HEADLESS_V1` markers.

### 2. Read-only target and ELF preflight — NO binary upload

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash tools/mpc-one/collect-over-ssh.sh --headless-check
~~~

The preflight must report `PASS: /run tmpfs ... shell CPU limit, SCHED_OTHER`, `NECESSARY CHECKS PASS: ELF...`, and `PREFLIGHT ONLY: nothing uploaded or executed on MPC`. Any missing library, different board/firmware, unexpected scheduling policy, stale target binary, or unavailable protection **blocks** execution. Do not install packages or change the MPC's system configuration to work around errors.

### 3. Opt-in headless computation on the actual MPC

Only if stage 2 passed, and after saving any open factory project and ensuring you are **not recording or playing**, run:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey

MPFREE_ACCEPT_VOLATILE_EXEC=YES MPFREE_ACCEPT_HEADLESS_EXEC=YES \
  bash tools/mpc-one/collect-over-ssh.sh --headless-smoke
~~~

The **two explicit opt-in variables** are required for this new M6 mode. It streams no more than 2 MiB to a uniquely created temporary directory in RAM-backed `/run`, verifies its SHA-256, executes the self-test and removes it on normal exit. It does **not** change the service, replace the factory UI, remount storage, flash anything, or claim the hardware.

Expected markers (measured numbers must not be invented):

~~~text
MPFREE_VOLATILE_STAGE_SHA256_OK
STAGE=EXECUTE_RUST
MPFREE_HEADLESS_V1
os=linux
arch=arm
backend=null
sample_rate_hz=44100
frames=88200
groove_events=12
sampler_notes=4
groove_voice_steals=0
sampler_voice_steals=0
prepared_bytes=94628
peak_abs=...
checksum_fnv1a64=...
elapsed_us=...
peak_rss_kib=...
rss_kib=...
status=PASS
STAGE=RUST_EXECUTED
MPFREE_HEADLESS_RUST_WATCHDOG_5S_ARMED
~~~

The owner measured `headless_audio_seconds=2.000` and `headless_offline_throughput_x=65.43`; this is a **batch-processing ratio**, not an xrun-free playback guarantee. The `peak_rss_kib` high-water mark includes shared libraries and thread stacks but is not PSS; the synthetic sample bank alone was ~94 KiB on x86-64.

On successful completion, verify that the **factory touchscreen, pads and sound still work**. Retain private reports in the ignored `research/` folder, with identifying information redacted before sharing. The launcher does not intentionally modify anything outside RAM-backed `/run`, but system SSH/PAM logging may independently make ordinary writes; an interrupted SSH connection may prevent immediate cleanup. Avoid forcibly killing the device.

**If the preflight or execution fails, do not bypass a safety gate or rerun repeatedly.** The host report includes bounded stage-by-stage diagnostic output. We should diagnose the first failure before the next attempt.

### Next milestone after successful M6 device test

Collect actual per-core CPU and process memory pressure while the stock app continues to run. Then design a **separately supervised longer-lived MPFree process** with a bounded control/event loop, `NullAudioBackend`, CPU and RAM budgets and user-visible cancellation. This requires more robust supervision than the current in-process watchdog before any exclusive resource handoff. Independent factory recovery remains mandatory before stopping `acvs.service`.

## Road ahead

1. Validate the bounded M6 integration and cross-compile against ARMv7 (this PR).
2. Add a specifically qualified, opt-in and reversible **M6 on-device null-backend launcher**, then measure the device's runtime, RSS/PSS and CPU-time while stock runs.
3. Introduce a separately supervised, longer-lived process architecture: command queue/event loop, safety limits, null audio and explicit stop. Non-RT file/network loading stays outside the render path. Minimize privileges; avoid root for general application logic.
4. Design device HAL adapters (ALSA, MIDI, DRM/touch, pad controls) **behind hardware-ownership gates**. Do not open factory-owned endpoints or stop `acvs.service` until independent factory restoration has been verified.
5. Only in the long term explore our own MPC-specific Linux **distribution**, preserving proven recovery and current compatible kernel/drivers unless clear engineering evidence supports replacements.

**Core user goal:** replace the factory music application with MPFree — first coexistence, later a validated reversible handoff. No need to replace Linux itself.
