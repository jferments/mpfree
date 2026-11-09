# M7: null-audio timing and independent HOST supervisor foundations

**Status:** Owner's physical MPC One **M6 test passed** (2026-11-09). M7 is a **host-only** benchmark and independently supervised process prototype with ARMv7 cross-build qualification. **M7 is not authorized for hardware deployment and does not add a device launch mode.** It does not stop, restart or signal the factory MPC application, take device ownership, or install a persistent service.

## M6 on actual hardware — observed facts (do not confuse with host CI)

Owner-run physical MPC execution report: `mpc-one-headless-execution-20261109T154352Z-3P2PZJ.txt`, SHA256 of report: `ea52990b9c07958f1170015374a5c8a11f98324f29d3f5d8d6c654491893bd59`. Private raw report **not committed**. Preflight report SHA256: `939da1a207baecc2b8a637ee95a930a881983210b10bb64f211d373c0807d931`.

| Measurement | MPC One (owner-run M6) | What it actually establishes |
| --- | --- | --- |
| Environment | Original MPC One, az0x 5.0.8, glibc 2.39 | Standard ARMv7 Rust ELF and shared libc work |
| ELF | 543,628 bytes; max required GLIBC_2.34 | Dynamic-loader/ABI preflight and on-device execution passed |
| Input | Synthetic generated PCM/WAV only | Does not load a commercial sample bank or claim physical audio |
| Frames/rate | 88,200 at 44,100 Hz | Exactly 2 seconds of generated stereo audio |
| Groove events, notes | 12, 4 | Real sequencer + sampler execute together |
| DSP checksum | `3dc4d6dabbb17fbd` | Matches previous x86-64 workstation and GitHub CI reference |
| Offline runtime | **30,565 microseconds** | **65.43× real-time throughput** on this light fixture |
| RSS / peak RSS | **1,792 KiB** each (~1.75 MiB) | Short-lived process memory including shared mappings, *not* actual MPFree DAW memory budget |
| Protection | verified SHA-256, stock service live, SCHED_OTHER, Rust watchdog | Reversible RAM-only sample computation; not proven independent recovery |
| Output | `backend=null`, `status=PASS`, cleanup reported | No factory-owned ALSA/MIDI/DRM/input device handles or service handoff |

**Do not claim that 65.43× offline performance means 65.43× real-time polyphony.** The 128-frame ALSA period at 44.1k Hz is approximately **2.902494 ms**, not 30 ms; every buffer must meet its own deadline. A 2-second aggregate timing test says nothing about maximum callback duration, queueing, core migration, fault latency or hardware xruns.

This successful test confirms *computation* alongside the stock application. The owner should independently verify factory touchscreen, pads, projects and sound after the trial; the report only verifies the factory process active before/after and contains no full physical-control regression.

## M7 design: measure timing tails before audio ownership

The new `rust/mpfree-session` is a finite Rust **synthetic callback** test using the exact same `HeadlessSession` and `NullAudioBackend` already tested under M6.

- 44,100 Hz, 128 frames per callback, 88,200 frames (one bar / two seconds), **690** calls including the final partial block.
- Uses an **absolute monotonic wall-clock schedule**, with deadlines calculated by integer nanoseconds from absolute frame positions. This eliminates accumulation of integer/period rounding drift in synthetic release times.
- Uses `std::thread::sleep` before each call; the callback itself executes the existing GrooveEngine, Sampler, mixer and null sink with preallocated buffers.
- Measures per-call CPU/wall execution time (**p99**, **maximum**), late release starts and worst wakeup lateness, number of completed buffers after their synthetic deadline, total rendering time and the end-to-end wall duration.
- Runs without real-time scheduling privileges and never opens an audio device. Host scheduler, timers and background load influence the measured deadline misses. **Do not interpret as a real ALSA callback or validated absence of xruns**.
- The `--offline-check` path renders the same 2 seconds **without pacing**, using exact known PCM hash `3dc4d6dabbb17fbd` as a deterministic regression invariant. Only the paced path reports meaningful synthetic misses/late starts.
- The `--self-test` path is paced at approximate playback speed (about two seconds of wall time). It carries a 7-second **in-process** safe-Rust watchdog. A true DAW still needs an independent supervisor.

## External host-only parent supervisor prototype

`tools/mpc-one/host_null_session_supervisor.py` independently starts exactly the local, already-built `mpfree-session --self-test` as a child process. It sets a separate process group, uses a bounded parent deadline and terminates the group on timeout. No user-provided binary, arbitrary command, SSH address or MPC target is accepted. It parses/version-checks the child's output and rejects a missing internal watchdog, wrong `backend=null`, unexpected engine counts, invalid timing fields and duplicate output markers.

This is a useful **host-side prototype** of supervisory separation, but it has not proven kernel scheduling, cgroups, isolated-CPU behavior, parent crash recovery or safe resource handoff on the MPC. The separate parent is currently on ML-tower, not the MPC.

## Owner validation on ML-tower (after merge)

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main

cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked

cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-session

# Fast deterministic engine regression; no pacing or hardware device.
rust/target/release/mpfree-session --offline-check

# Approx. 2-second synthetic callback schedule, then external parent validation:
python3 tools/mpc-one/host_null_session_supervisor.py

# Optional: cross-build M7 (host only), without copying to hardware.
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-session
~~~

Expected host-parent fields:

~~~text
MPFREE_M7_HOST_PARENT_SUPERVISOR_V1
supervised_child=mpfree-session
hardware_access=none
child_watchdog=armed
frames=88200
callback_count=690
deadline_misses=...
late_starts=...
max_callback_us=...
p99_callback_us=...
max_start_lateness_us=...
total_render_us=...
wall_elapsed_us=...
checksum_fnv1a64=3dc4d6dabbb17fbd
status=PASS
~~~

**Do not execute `mpfree-session` on the MPC yet.** There is no on-device M7 launcher or dedicated safety preflight in this milestone; the validated M5/M6 modes remain unchanged.

## Next hardware-safe acceptance gates

1. Verify the M7 native and ARMv7 build on CI, independently review the parent/child safety model and create a *separately allowlisted* M7 RAM-only launcher—only with explicit owner opt-in.
2. Measure callback timing distribution at idle and under typical factory-app playback, with **no hardware device opened by MPFree**. Compare p99/max render duration vs 2.90ms synthetic budget and distinguish wakeup latency from DSP cost.
3. Implement a persistent **separate on-device supervisor** with explicit stop/cancel, privilege reduction, stable resource budgets, logging outside realtime threads, and fail-closed timeouts. No auto-start or service activation yet.
4. Build sample-bank stress fixtures, track count/polyphony and longer (minute-scale) benchmarks. Real-world, memory-heavy loads will be substantially different from current 94 KiB synthetic content.
5. Independently verify full factory recovery before any real acvs.service stop/restart or exclusive audio/MIDI/DRM/input takeover. Longer-term custom Linux distribution remains optional.
