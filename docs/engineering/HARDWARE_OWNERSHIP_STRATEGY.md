# MPFree ownership strategy: recover hardware resources first; custom Linux distribution later

**Status:** Accepted long-term technical direction, staged execution plan only. No real factory-app stop, bootloader edit, firmware flash, repartition, privileged takeover, alternate-boot attempt or custom-OS installation is implemented or authorized here.

**Purpose:** Distinguish **application resources**, **ordinary Linux overhead** and **intentional realtime CPU isolation**. Deliver a fully MPFree-owned music-production environment on the original MPC One while preserving a credible route to restore factory firmware after *every* planned experiment.

See the [sanitized owner-run hardware/ABI/stock-resource survey](../hardware/MPC_ONE_3_6_0_ABI_HANDOFF_SURVEY_20261109.md), [ARMv7 compatibility](ARMV7_USERSPACE_COMPATIBILITY.md), and [reversible handoff gates](REVERSIBLE_HANDOFF.md).

## What the newest MPC measurements actually mean

| Observed item | Value | Interpretation and caution |
| --- | --- | --- |
| Linux RAM | `MemTotal: 2067188 kB` (~2,019 MiB) | Linux-available physical memory, not every byte of DRAM on the board |
| Factory process `VmRSS` | ~1,043,768 kB (~1,019 MiB) | **Factory application**, not OS kernel reservation. Includes file/shared pages; approximate |
| Factory process `VmLck` | ~1,040,052 kB (~1,016 MiB) | Application's **locked** mappings. May be released with process exit, but reclaim must be measured, not projected from RSS alone |
| `MemAvailable` | ~882,348 kB (~862 MiB) | Kernel's current estimate of allocatable memory without swapping; dynamic |
| Swap | 0 | **Do not** budget for swap under pressure |
| Process threads | 38 | Each `/proc/PID/task/TID/status` repeats the process's whole VmRSS; do not multiply by 38! |
| CPU topology | cores 0–3 online, cores 2–3 isolated | Isolation restricts general scheduler balancing, **not** use by application code assigned via affinity |
| Factory audio | `Audio Processing` pinned to CPU3; `AudioWorker0/1/2/3` pinned to CPU0/1/2/3 | Factory actually uses **all four cores**; a CPU affinity is not a percentage of occupied CPU time |
| ABI | GNU libc release banner **2.39**, ARMv7 EABI5 hard-float loader and ALSA/DRM/EGL | Now explicitly observed on the device; **binary-specific GLIBC symbol and dynamic-library matching still required** |

**Correction to the "60% RAM / 25–50% cores reserved by the OS" interpretation:**
- The stock application itself currently accounts for roughly **half** the Linux memory, not the Linux kernel. Eliminating that application could free much of the footprint with the **same vendor kernel and drivers**.
- CPU isolation is an intentional realtime scheduling technique. The core assigned the main audio callback is **CPU3**; supporting audio-worker threads run on all four cores. No fixed 25–50% tax can be inferred. Actual runtime utilization must be measured in *CPU time*, under representative workloads.
- There **is** operating-system overhead (kernel memory, housekeeping services, interrupts, context switches, page cache), but existing reports don't isolate its precise share. The biggest early optimization may simply be to remove the heavyweight factory user application and unnecessary services while retaining Linux and its drivers.

Official references:
- Kernel CPU isolation: https://docs.kernel.org/admin-guide/cpu-isolation.html
- Per-task CPU policy and runtime: https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html
- Linux resident/locked memory: https://man7.org/linux/man-pages/man5/proc_pid_status.5.html

## Four-layer target design

~~~text
                 MPC One physical hardware
  Rockchip RK3288 | Mali/Panfrost | ACVA audio | MIDI | touchscreen
                            |
  ┌───────────────────────────────────────────────────────┐
  │ L0 Boot/recovery: FACTORY-RESTORABLE by verified tools│
  │ Never modify boot ROM; avoid eMMC bootloader writes   │
  ├───────────────────────────────────────────────────────┤
  │ L1 Linux kernel/device drivers                        │
  │ Start with vendor 6.6 PREEMPT_RT and board DT         │
  │ Keep ACVA/audio/MIDI/touch/DRM quirks functioning      │
  ├───────────────────────────────────────────────────────┤
  │ L2 Minimum base userspace / services                  │
  │ Phase A: vendor az0x; future: reproducible MPFree     │
  │ system image (Yocto/OpenEmbedded may be natural)      │
  ├───────────────────────────────────────────────────────┤
  │ L3 100% MPFree-owned Rust music-production environment│
  │ Sequencer, sampler, mixer, UI, networking, plugin FFI │
  └───────────────────────────────────────────────────────┘
~~~

**"Custom OS" should mean owning L2 + L3 first, while reusing L0/L1 only as long as those components remain useful and redistributable.** An independent userland is a legitimate custom Linux distribution. Writing a replacement kernel or bootloader is not necessary to eliminate the factory application's 1 GiB process.

## Sequential reversible development gates

### Gate 0 — Passive board/resource understanding (mostly completed)
Read current DT/kernel/systemd ownership/ALSA/MIDI/USB/DRM, native ABI, stock memory and per-thread affinity. Do not open factory-owned PCM, display or MIDI clients. Capturing private reports is okay only locally with identifiers redacted before publication.

**New read-only observation:** `--capacity` takes two snapshots of `/proc/stat` and factory per-thread CPU ticks over two seconds, plus `/proc/meminfo`, process `/proc/PID/status` and at most one `smaps_rollup` read. Use the host-only report analyzer, never a guessed "% of core reserved" derived from affinity:
~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash tools/mpc-one/collect-over-ssh.sh --capacity
python3 tools/mpc-one/analyze-capacity.py \
  "$(ls -t research/mpc-one-capacity-*.txt | head -n 1)"
~~~

The script never changes affinity, thread priority, services or mounts. Run once at idle and once during ordinary playback/sampling for a more meaningful comparison. **Brief CPU-counter measurements are not a hard-realtime latency proof**.

The revised `--rtthreads` also reads `/proc/PID/task/TID/stat` for numeric scheduling policy, RT priority and last CPU because the earlier `/sched` text survey lacked those fields. This remains read-only.

### Gate 1 — Establish binary compatibility without touching device resources (next)
1. Build a very small standalone ARMv7 hard-float Rust binary on ML-tower using a pinned, reproducible toolchain.
2. Run `tools/mpc-one/check-arm-abi.py` locally against the confirmed **glibc 2.39** report, PT_INTERP and *every* linked `DT_NEEDED` name. Fail closed on mismatches.
3. Audit the exact ELF dependencies, CPU features and code paths for the smoke binary.
4. After a separate owner-reviewed test command, copy the binary to **volatile `/run` RAM storage only**, never `/etc`, `/var`, `/data`, factory rootfs, boot partitions or content.
5. Execute only a benign self-test (print version/CPU features, perform bounded memory arithmetic, exit). Use timeouts, resource limits, SHA-256 verification, automatic deletion and a guarded transport connection.
6. Confirm that the factory UI, audio, pads and network MIDI continue operating normally throughout; never acquire their hardware endpoints.

**Important distinction:** a harmless userland smoke binary does **not** require us to stop the stock app or test factory takeover. However, it still needs an explicit runbook and fail-safe staging. We will not run it automatically from a research script.

### Gate 2 — Independent MPFree session without exclusive resource handoff
Run host-side state/control and selected bounded algorithms in volatile memory while factory app remains active, with strict memory and CPU budgets. Measure working set, thread scheduling, library compatibility and crash behavior. Do not bind another process to the factory's exclusive audio/DRM/MIDI devices.

### Gate 3 — Restore-before-stop proof and controlled factory userland handoff
**BLOCKED** until the Tier 1 recovery gate passes (adopted by the owner 2026-11-09; see the [recovery strategy](RECOVERY_AND_OS_STRATEGY.md)): a verified full off-device eMMC backup, measured zero writes to `p1`–`p6` across the session, and a stub stop → start rehearsal with no MPFree code. This replaces the earlier requirement of a proven unbootable-device restore for RAM-only sessions. Any persistent change still needs Tier 2 or Tier 3.

With that proven, separate volatile watchdog, explicit per-session owner consent, factory saved and quiescent, no recording, and failure-handling tested on a fake service graph, attempt only the narrowest factory-app release. Observe exact device ownership, run MPFree as a session child and guarantee *best-effort* restoration. A watchdog alone cannot recover from an unbootable device.

Start by proving stock app restore **without** starting our audio engine. Later test the PCM, MIDI, DRM and touchscreen resources independently, with strictly bounded timeouts and owner-visible cancellation.

### Gate 4 — Optional independently booted MPFree userspace
Testing another kernel in RAM only (kexec, and possibly fastboot or MaskROM RAM boot) leaves the eMMC untouched and is research under the [recovery strategy](RECOVERY_AND_OS_STRATEGY.md). Anything persistent needs the **Tier 3** gate: MaskROM recovery proven on this unit and an eMMC read-back verified, plus exact understanding of the vendor boot flow ([boot map](../hardware/MPC_ONE_3_6_0_BOOT_MAP_20261109.md)). Only then explore an alternate rootfs that does not overwrite factory eMMC. The ability to boot from USB, SD, RAM or recovery transport must be **measured**; it is not established by current documentation. A separate U-Boot/extlinux slot or removable boot medium should not be claimed functional until demonstrated.

Only then optimize distro contents: include minimal open Linux base system, ALSA/DRM/MIDI/evdev, Rust MPFree, reliable watchdog/recovery tools and verified updates. Evaluate whether retaining vendor kernel and proprietary driver components is legally/distribution-wise permitted; separate user-owned extraction from redistributed binaries.

### Gate 5 — Replace kernel / boot chain only if justified
**Not a default goal.** Replacing working vendor PREEMPT_RT drivers can remove necessary codec/ACVA/MIDI/display support and materially raises brick risk. Do this only if verified kernel bugs, missing features, security, upstream availability or long-term maintenance demands outweigh those costs; validate boot/recovery *first*.

## First resource-performance targets (proposals, not observed)

| Metric | Initial engineering target | Why |
| --- | --- | --- |
| Base MPFree process and services | Measure first, then aim for a **<256 MiB idle budget** | Ambitious but testable against 1,019 MiB stock resident footprint |
| Prepared WAV sample bank | Keep **64 MiB** initial cap | Already implemented; avoid swapping on a 2 GiB target |
| Audio callback | Measure worst-case vs **128/44100 ≈ 2.90 ms** ALSA period | Need scheduling headroom, not an average-only benchmark |
| CPU | Report actual per-core occupancy under load | Affinity/isolation ≠ consumption |
| Recovery | All persistent-boot experiments gated on proven independent restore | No unattended chance of losing the factory environment |

The final goals should be workload-specific: 16-track/32-track sequencer mixes, sample-based polyphony and external-DSP network streams. A smaller RAM footprint is only a win if it preserves sound quality, low xruns, functional features, safe recording and stable recovery.

## Immediate next actions

- Run the passive `--capacity` once idle, once under a normal CPU-intensive MPC pattern, then compare actual **percent busy** and factory process CPU tick consumption.
- Run revised `--rtthreads` to capture **SCHED_OTHER/FIFO/RR**, per-task RT priority and last CPU from `stat`, not just affinity.
- Prepare a carefully bounded **RAM-only ARMv7 hello-world self-test** after proving binary-specific glibc compatibility. No service changes.
- Research and independently test **factory firmware recovery** before touching any bootloader, recovery partition, rootfs, or factory app/service resources. The tier each kind of change needs is in the [recovery strategy](RECOVERY_AND_OS_STRATEGY.md).

**Safety rule:** All measured CPU/memory savings are hypotheses until MPFree actually replaces the factory user process under a proven reversible procedure. No current GitHub code has performed that takeover.
