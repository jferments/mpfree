# M7: optional synthetic callback timing on the physical MPC (no ALSA)

**Status: PHYSICAL MPC M7 PASSED.** On 2026-11-09 owner ran read-only `--session-check` and double-opted-in to finite RAM-only `--session-smoke`, obtaining correct PCM checksum and zero synthetic deadline misses while factory app was active. This is **not** a real ACVA ALSA timing/xrun test, a persistent daemon, an application restart or a graphics takeover. M5 and M6 modes are unchanged. Full sanitized measurements: [validated M7/process/touch report](m07-validation-record.md).

## Physical MPC One M7 timing — owner-run 2026-11-09

All live preflight gates passed: correct original MPC One / az0x 5.0.8 / libc 2.39, ELF32 ARM hard-float, target PT_INTERP, highest symbol need GLIBC 2.34, no missing libraries, /run tmpfs, stock application active, SCHED_OTHER, independent double owner consent, SHA-256 verification and volatile cleanup trap.

**Real MPC, still pure userspace/null backend:**

| Metric | Observed |
| --- | ---: |
| Frames / callbacks | 88,200 / 690 |
| Nominal 128-frame period | 2,902.49µs |
| Synthetic deadline misses | **0** |
| Late starts | 690 (all slightly behind nominal start) |
| p99 / maximum callback duration | **66µs / 165µs** |
| Maximum start lateness | **167µs** |
| Sum of render times | **26,859µs** |
| Elapsed wall | **1,999,934µs** |
| PCM FNV1a64 | **`3dc4d6dabbb17fbd`** — host+M6 parity |
| Outcome | **PASS** |

This means our ~2-second **small fixed sample/sequencer** workload produced all synthetic callbacks inside their deadlines while the factory application remained alive. The slowest callback consumed 5.68% of a nominal 2.90ms period; total render time was ~1.34% of a single core-equivalent wall-time budget. These are **not** whole-DAW CPU utilization or an xrun-free ALSA guarantee. No DRM, evdev, controller MIDI or ACVA device was opened by M7.

Original private user reports remain outside the repository. No user identifiers or raw logs are committed.

## M7 owner ML-tower result — 2026-11-09

The owner executed all Rust workspace tests successfully, cross/machine-independent offline PCM regression and host parent supervision:

| Host test mode | Observed |
| --- | --- |
| offline | 690 blocks, 88,200 frames, same PCM checksum `3dc4d6dabbb17fbd`, max callback **11 μs**, p99 **6 μs**, 0 deadline misses (not paced), total render **2,453 μs** |
| wall-paced, parent supervised | 690 blocks, **0 synthetic deadline misses**, all 690 callback starts reported late by small scheduling amounts, maximum wakeup lateness **81 μs**, max callback **28 μs**, p99 **10 μs**, total render **3,036 μs**, elapsed **1,999,889 μs** |

**These are x86-64 workstation measurements**, not on the MPC. A scheduler thread that calls `sleep` can wake a little after its intended release; the benchmark records all late starts even when they are far inside the 2.90ms period. Fast rendering and zero *synthetic* misses do **not** prove the actual ALSA engine would be xrun-free.

## Guarded M7 hardware options

- `--session-check`: existing exact Ethernet, route, MAC/SSH identity; reads live original MPC One az0x 5.0.8 / glibc2.39 and RAM safety prerequisites, checks that `mpfree-session` is an allowlisted ARMv7 hard-float ELF with compatible dynamic loader/libs and contains the correct M7 internal watchdog/version markers. **No upload and no execution on MPC**.
- `--session-smoke`: explicitly requires **both** `MPFREE_ACCEPT_VOLATILE_EXEC=YES` and `MPFREE_ACCEPT_SESSION_EXEC=YES`, then repeats the preflight, verifies `acvs.service` is active, that `/run` is tmpfs, the caller inherited `SCHED_OTHER` and supports `ulimit -t 3`. Hashes the binary locally and after transfer; executes exactly `mpfree-session --self-test` in a fresh RAM-only temporary directory; 7s internal Rust watchdog + host 18s SSH deadline; removal shell trap.
- The profile requires `backend=null`, `pacing=monotonic`, 128-frame period, 88,200 frames, 690 callbacks, 12 groove events, four sampler notes, exact PCM hash and a unique watchdog marker. It verifies timing fields and preserves **nonzero** deadline misses as measured results rather than suppressing them.

All hardware devices remain owned by factory Akai; M7 does **not** open `/dev/snd`, `/dev/dri`, `/dev/input`, GPIO, MIDI, I²C or a display, and does not send any stop/start/restart/kill signal to factory processes.

### Runbook on ML-tower (after CI and merge)

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main

cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked

# Build the exact new, watchdog-equipped ARMv7 target executable.
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-session

ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519

# FIRST: read-only board, ABI, RAM, scheduler and ELF verification.
bash tools/mpc-one/collect-over-ssh.sh --session-check
~~~

**Stop immediately if the preflight fails**; do not install packages or change device configuration to override gates. Save the Akai project and do not record or play while running synthetic timing.

### Separately authorized finite test

~~~bash
MPFREE_ACCEPT_VOLATILE_EXEC=YES MPFREE_ACCEPT_SESSION_EXEC=YES \
  bash tools/mpc-one/collect-over-ssh.sh --session-smoke
~~~

Successful execution will show the separate `MPFREE_SESSION_V1`, `MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED` and `status=PASS` markers, exact null/PCM reference counts, measured `deadline_misses`, `late_starts`, `max_callback_us`, `p99_callback_us`, `max_start_lateness_us`, `total_render_us`, `wall_elapsed_us` and reported RAM-only cleanup.

A deadline miss is **not** necessarily a failed test: it reveals wakeup delay and/or render timing. A failed safety gate, executable checksum, watchdog, factory service or PCM checksum **is** a failed test. Inspect private redacted report first before retrying.

The last step is to verify that factory touchscreen, pads, audio and GUI still work after this null-only session. An external physical recovery procedure remains a separate prerequisite for any later stop/restart of the factory application.

## What the M7 numbers cannot prove

- No physical ACVA ALSA callbacks, hardware interrupt deadlines or xruns are measured.
- No GUI framebuffer/DRM modesetting or touchscreen evdev access is attempted.
- A 2-second instrument fixture with ~95 KiB prepared samples is not a representative full project/polyphony/memory stress test.
- The Rust internal watchdog is *not* an independent on-device stock-service restoring supervisor.
- A successful stock service state before and after does *not* establish recovery after an intentional factory shutdown.

Next: analyze M7 callback timing on real ARM; continue read-only process/touchscreen hardware surveys; then design a separate privileged supervisor with independent verified boot recovery before any real screen/ALSA handoff.
