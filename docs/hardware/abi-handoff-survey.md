# MPC One firmware 3.6.0: observed az0x ABI and factory handoff properties (2026-11-09 UTC)

**Sources:** Owner-run identity-gated read-only surveys on an original MPC One, using the existing temporary SSH access. The unmodified raw reports remain **only on the developer workstation**, excluded from Git. Their SHA-256 fingerprints allow local verification without publishing identifying boot arguments.

| Source | SHA-256 of raw local report |
| --- | --- |
| `mpc-one-abi-20261109T073912Z-55Jk6w.txt` | `4e1155cc7f559eda9b8579eacbbd50b337c7cc2d51b43a95e456eba92d93d2f8` |
| `mpc-one-handoff-preflight-20261109T073912Z-Plo0RY.txt` | `4c7b7c491fbe7231ef41e0ee6d6d05c18a0d78ab91cf8a50489089939e4e3cd0` |
| `mpc-one-abi-20261109T080714Z-L6g2wV.txt` (explicit libc release banner) | `85bc82f87687e2e9bee029b7954ae9b6e5782b5494b176cb114b9a5f7ab258e2` |
| `mpc-one-rtthreads-20261109T080714Z-0A3BwY.txt` (factory task affinity) | `3e108a9aae4ad09ba4f7a1848799aa864b48e44c162ef9e7ce2919a03ee9142b` |

**Publication rule:** This document contains sanitized technical observations only. No boot PARTUUID, unique ID, MAC address, account path, SSH keys, IP addresses, raw console logs or exact device-report attachment is committed.

## Verified user/workstation development status

- Local checkout switched to `main` at squash-merge `621d5de`.
- Owner's local Rust 1.97.1/Cargo build completed successfully.
- Host tests: **5 click + 6 groove + 4 network + 5 WAV sampler = 20 library tests passed**, no failures. Offline user-WAV sampler preview produced 88,200 frames, 44.1-kHz stereo S32_LE, one sample loaded, zero voice steals.
- Both new read-only MPC reports completed successfully over the identity-verified SSH link.

## Board/OS/ABI observations

| Parameter | Direct observation | Interpretation |
| --- | --- | --- |
| CPU | ARMv7 CPU part `0xc0d`; four repeated cpuinfo processor records; `neon vfpv3 vfpv4 vfpd32 idiva idivt` | Rockchip RK3288 Cortex-A17 family, hard-float with NEON present |
| Linux | `6.6.95-az01-2025-06-27-rt55`, PREEMPT_RT | Native realtime-capable kernel; actual scheduler configuration below |
| Distribution | az0x `5.0.8 (scarthgap)` | Yocto-derived runtime |
| Dynamic loader | `/lib/ld-linux-armhf.so.3` exists | Expected ARM hard-float PT_INTERP available |
| C runtime | `/lib/libc.so.6` and `/lib/libm.so.6` exist | GNU libc present, exact version **not yet explicitly reported** |
| Version tags in libc strings | `GLIBC_2.35` through **`GLIBC_2.39`** in last-five sorted entries | Strong evidence of 2.39-era ABI, **not a conclusive libc release version or exported-symbol audit** |
| Library SONAMEs present | `libasound.so.2`, `libdrm.so.2`, `libEGL.so.1`, `libGLESv2.so.2`, `libgbm.so.1`, `libstdc++.so.6` | ALSA, DRM and GPU userspace building blocks exist; exact APIs, driver stacks and compatibility remain unverified |
| Build tools on MPC | `readelf`, `objdump`, `ldd` and `pkg-config` absent; `strings` present | Perform ELF inspection and linking on developer host; do NOT install build tools on MPC |

**External corroboration:** The official [Yocto 5.0 Scarthgap release notes](https://docs.yoctoproject.org/5.0/migration-guides/release-notes-5.0.html) specify **glibc 2.39** as their baseline. Combined with this vendor-derived az0x distro's Scarthgap declaration and observed `GLIBC_2.39` string, this strongly increases confidence that the installed libc is 2.39-based. **A custom inMusic build can still differ from the default Yocto recipe**; the exact release remains explicitly unverified until the on-device libc banner or target ABI is independently inspected.

**Correction to the earlier checker:** the original `check-arm-abi.py` aborted without `getconf`, which is absent on the device. A revised checker now distinguishes **explicit getconf/release-banner confirmation** from the weaker `GLIBC_*` string ceiling. If only the strings are present, it provides a useful explanatory *NOT QUALIFIED* result (exit 4) rather than converting 2.39 into an asserted release version. An updated `--abi` survey tries to read an embedded GNU libc release banner **as read-only text** without executing libc. See [ABI comparison runbook](../research/armv7-userspace-compat.md).

## Factory service graph and lifecycle

**Snapshot**, not a guarantee after a firmware upgrade:

| Unit | Type | Restart | Stop timeout | Kill mode | Relationships |
| --- | --- | --- | --- | --- | --- |
| `acvs.service` | `simple` | `always` (100ms delay) | 90 s | `control-group` | `Wants=az01-script-runner.service`; ordered `After=az01-network-midi.service` and script-runner, firmware-update helpers |
| `az01-network-midi.service` | `simple` | `always` (100ms delay) | 90 s | `control-group` | Network MIDI independent, ordered before `acvs` |
| `az01-script-runner.service` | `simple` | `no` | 90 s | `control-group` | Wants network via `Requires=network.target`, ordered before `acvs` |

**Implications / not-yet-tested assumptions:**
- `After=` encodes *start/stop ordering*, not a requirement or automatic service-stop cascade; stopping all three helper units is **not** justified by observed information.
- A deliberate systemd service stop is exempt from `Restart=always` auto-restart, while direct process killing may immediately restart the factory. That does not make deliberate stop safe yet.
- `KillMode=control-group` and the long 90-second stop timeout mean any later restoration supervisor must respect orderly service shutdown, task groups and actual timeouts, not blindly kill the factory process after a short delay.
- The factory executable path resolves to `/usr/bin/MPC` even though systemd starts `/usr/bin/az01-launch-MPC`. Any exact allowlist has to model the launcher/executable relationship, not a hardcoded PID.

## Resource ownership, per-process affinity and memory

The observed factory main process held:

- **Playback:** `/dev/snd/pcmC1D0p`, ACVA card1 device0 subdevice0.
- **Capture:** `/dev/snd/pcmC1D0c`, same ACVA card/device.
- **MIDI:** `/dev/snd/seq` and `/dev/snd/midiC0D0`.
- **Mixer/control:** `/dev/snd/controlC1`.
- **Display/terminal:** `/dev/dri/card0`, `/dev/tty0`.
- **Input:** `/dev/input/event0` (touch) and `/dev/input/event1` (GPIO).

Both ALSA playback and capture reported `subdevices_count=1` / `subdevices_avail=0`. Stock software presently has exclusive use; **do not open a competing PCM endpoint during normal use**.

The same process reported `Threads=38`, leader-task `Cpus_allowed_list: 0`. The kernel command line shows `isolcpus=2-3`; do not publish the full command line because it contains persistent boot identifiers. **This is NOT evidence that every thread is bound to CPU 0.** Per-task affinity and SCHED_FIFO/RR policies are unmeasured. New passive `--rtthreads` mode addresses that gap.

Memory snapshot:

| Field (KiB) | Observed |
| --- | ---: |
| `MemTotal` | 2,067,188 |
| `MemAvailable` | 882,348 |
| `SwapTotal` | **0** |
| Factory `VmRSS` | 1,043,768 |
| Factory `VmLck` | 1,040,052 |
| System `Mlocked` | 1,039,452 |

**Practical implication:** the factory locks roughly 1 GiB of RAM while running. MPFree should not assume a desktop-style memory budget, oversubscribe RAM during coexistence, or assume all lockable memory is released immediately after the factory process exits. 64 MiB prepared-sample caps remain conservative; monitor complete process memory + page cache before allocating audio pools. System available memory is a snapshot, not an MPFree allocation guarantee.

Storage snapshot: root `/dev/root` mounted read-only; `/run` and `/tmp` are tmpfs; `/data` is read-write; **`/etc` and `/var` are persistent read-write overlays backed by `/data`**. **A tool touching those overlays is not volatile.** eMMC partition and boot metadata remain unchanged by the surveys.

## Subsequent owner-run evidence: explicit glibc 2.39 and all-core factory workload

The second owner-run `--abi` read-only survey **explicitly identified the installed C library**:

~~~text
GNU libc release banner: GNU C Library (GNU libc) stable release version 2.39.
~~~

This is **stronger than merely observing a `GLIBC_2.39` symbol string**. The target device's libc declares release **2.39**, agreeing with the upstream Scarthgap baseline. Still verify each ARMv7 candidate binary's `PT_INTERP`, `DT_NEEDED`, versioned symbol needs and expected NEON/VFP support before execution. The dynamic linker file, ALSA/DRM/GBM/EGL/GLES and standard C++ library SONAMEs are present, not yet proven to be suitable for every candidate dependency.

The second owner-run `--rtthreads` survey lists **38 threads** across all four CPU cores. It reports:

| CPU allowed | Factory thread roles (selected) |
| --- | --- |
| CPU0 | Main thread, JUCE Message, JUCE MIDI Input, AudioWatchdog, AudioWorker0, GUI/file-browser |
| CPU1 | AudioWorker1, Background 1, Background_high, FileIO, directory watches |
| CPU2 | AudioWorker2, Background 2, MIDI Out, MIDI |
| CPU3 | **Audio Processing**, AudioWorker3 |
| CPU0–1 | sentry-http |

The presence of `isolcpus=2-3` therefore **does not mean 50% of CPU cores are occupied by or inaccessible to MPFree because of Linux**. The vendor intentionally places its own workers on isolated cores. CPU affinity is a set of eligible cores, not CPU time consumption; actual utilization is not measured in this report.

The initial scheduling probe did **not** show policy or RT priority for the threads (task `sched` debug fields were absent or not usable). The revised `--rtthreads` script now derives the Linux `policy`, `rt_priority`, and last-used CPU from each task's documented `stat` fields 39–41, requiring another **read-only** run to measure these values.

**Important memory accounting pitfall:** `VmRSS: 1043768 kB` appears in every task's `/proc/PID/task/TID/status` because the threads share the **same process address space**. It is **not** 1 GiB *per thread* and must never be multiplied by 38. The factory process has ~1,019 MiB resident and ~1,016 MiB locked, representing roughly **half the ~2,019 MiB RAM** in Linux; that is **factory application memory**, not Linux's reserved memory. A new `--capacity` probe reads `smaps_rollup` to distinguish proportional/private memory and samples CPU counters over two seconds to measure workload utilization rather than making assumptions from CPU affinity.

Systemd reported `LimitMEMLOCK=8388608` (8 MiB), while the stock application's `VmLck` was ~1,016 MiB. **This discrepancy is not resolved**; process privileges/capabilities, inherited locks or alternative memory-mapping APIs could matter, but should not be assumed as the cause. A read-only process capability and memory-map inspection would be needed before attributing the mechanism.

The long-term strategy is to reclaim hardware resources **first by replacing the factory application while reusing the working kernel/drivers**, then optionally build a minimal, independent MPFree Linux userspace only after recovery is proven. See [hardware ownership strategy](../research/hardware-ownership.md).

## Recovery blocker and next observations

These reports make the future hardware handoff design substantially more concrete, but **neither constitutes proof that recovery from an unbootable MPC works** or that systemd/ACVA/DRM/MIDI will restore after a stop. A real handoff is still blocked.

Safe next owner actions:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519

# Repeat ABI report, now attempting a read-only embedded release banner:
bash tools/mpc-one/collect-over-ssh.sh --abi

# Newly added: per-thread CPU affinity/real-time scheduling (read-only):
bash tools/mpc-one/collect-over-ssh.sh --rtthreads
~~~

Review both new reports for identifying metadata before sharing. Compare a *host-built* ARM binary using `tools/mpc-one/check-arm-abi.py` and the updated ABI report. See the completely side-effect-free Rust [handoff simulator](../research/reversible-handoff.md): it cannot affect factory services.

**Never send these reports to public GitHub unredacted. Do not stop/kill/disable/mask factory services or open their PCM/DRM devices.**
