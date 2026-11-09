# Owner-run validation: physical M7, factory service topology, ILI2117 and DSI (2026-11-09)

**Device:** original MPC One, factory software 3.6.0 / az0x 5.0.8, PREEMPT_RT kernel 6.6.95, ARMv7, 2GiB RAM.

**Source:** user-supplied private reports produced by the allowlisted MPFree scripts on 2026-11-09 UTC: `--process-map`, `--touchscreen`, `--session-check`, and double-opt-in `--session-smoke`. They were inspected separately; raw reports are **not** added to the repository. Identifying IP/MAC, transient PIDs, serials, private executable hashes, and boot arguments are excluded from this sanitized summary.

**Important limits:** reports are snapshots; the process survey capped the overall PID search at 180 and individual fd searches at 256. An unseen owner cannot be ruled out. No services stopped, framebuffer/modesets/input devices opened, I²C transactions made, or firmware changed. There has been **NO reversible factory app stop/restart test**.

## First physical M7 paced null-audio timing: PASS

| Field | Owner's actual MPC |
| --- | --- |
| Architecture / audio backend | ARMv7 / pure userspace `null` |
| Period / rate | 128 frames / 44,100 Hz (2.902ms) |
| Audio frames / callbacks | 88,200 / 690 (final partial callback) |
| PCM hash | `3dc4d6dabbb17fbd` — matches M6, desktop and CI |
| Synthetic deadline misses | **0** |
| Late starts | 690/690 |
| Max scheduling lateness | **167µs** |
| p99 render duration | **66µs** |
| Maximum render duration | **165µs** |
| Sum of render times | **26,859µs** |
| Wall elapsed | **1,999,934µs** |
| Status / cleanup | `PASS`; temporary `/run` stage removed on normal exit |

The paced benchmark completed while the factory MPC service was active and live ELF/ABI preflight passed (target libc 2.39; binary hard-float ABI; highest GLIBC symbol need 2.34). The RAM-only stage was SHA-256 verified and no factory playback/DRM/evdev device opened by M7.

At 128/44100s per buffer, the nominal deadline is **2,902.49µs**. The observed maximum callback of 165µs is **5.68%** of that period; p99 66µs is **2.27%**. Aggregate 26.859ms rendering / 2s wall = **1.34% of one core-equivalent wall time** for this tiny instrument fixture. These are workload-specific, not system CPU utilization or a future DAW polyphony guarantee.

All callbacks are classified as 'late' because `Instant::now()` happened after nominal release even when by microseconds; **zero deadline misses** means all synthetic buffer completions preceded their nominal deadlines. This measures userspace timer waking with `SCHED_OTHER` and no ALSA interrupts: **it does not establish actual ACVA ALSA xrun performance, real-time priority/affinity correctness, or GUI workload headroom**.

## Factory process topology: main app vs independent services

Snapshot confirmed:

| Linux systemd unit | Executable / role evidence | State / actual service behavior |
| --- | --- | --- |
| `acvs.service` | Main `/usr/bin/MPC` DAW process (38 threads); launched by `/usr/bin/az01-launch-MPC` | active; `Type=simple`, `Restart=always`, 100ms restart delay, `KillMode=control-group`, 90s stop timeout |
| `az01-network-midi.service` | **`DeviceControlServer.bin`** is a **separate 10-thread process**, with its own `/dev/snd/seq` fd | active; `Restart=always`; ordered before, but **not** explicitly required or PartOf main app |
| `az01-script-runner.service` | Separate script runner, and its cgroup also contained SSH helper/session processes | active; `Restart=no`; wanted and ordered after by acvs; should be kept running |
| `az01-power-button.service` | `az01-pwrbtn` is separate and **also opens `/dev/input/event1`** | active; `Restart=no`, provides emergency shutdown handler; must remain active |
| `az01-unsupported-device-daemon.service` | Separate DBus-backed unsupported-device checker | active; exact behavior not otherwise tested |
| `az01-usbsata-fixer.service` | JMicron USB-SATA configuration helper (actual ExecStart includes a firmware filename) | active; do not perturb storage service |
| `mpfree-sshd.service` | Separate MPFree SSH service and cgroup | active; potentially independent return/control channel; loss-of-connection faults still need simulation |
| `touch-fw-update.service` | Boot-time Ilitek firmware updater unit | active/exited; **do not rerun or change** |

The factory main process also has a child/session `crashpad_handler` in the `acvs` cgroup. Other factory threads include `JUCE Message`, `JUCE MIDI Input`, `AudioWorker0–3`, `Audio Processing`, file-browser/file-management and directory-watcher tasks. **They are threads in one address space; not independently stop/start-able services.**

**Observed device fd ownership while stock is running:**
- Main MPC: `/dev/dri/card0`, `/dev/tty0`, `/dev/input/event0`, `/dev/input/event1`, `/dev/snd/seq`, three `/dev/snd/midiC0D0` handles, ACVA `/dev/snd/controlC1`, playback `/dev/snd/pcmC1D0p` and capture `/dev/snd/pcmC1D0c`.
- DeviceControlServer: `/dev/snd/seq`.
- Power-button service: `/dev/input/event1`.
- No verified independent compositor was found in the process map. This is consistent with direct DRM/KMS rendering from the MPC application, **not proof** that every graphics interaction is inside it.

**Implication:** the future **whole-main-application** handoff may leave networking, controller service, script runner and power button alive. This is consistent with systemd unit metadata (`After=`, `Wants=` but no `Requires=az01-network-midi`, no `PartOf=`), **not** a demonstrated actual post-stop state.

## Actual installed ILI2117 input driver and multitouch bits

- Input name `ILI2117 Touchscreen`, bus ID `0018` (Linux BUS_I2C), sysfs `/sys/bus/i2c/devices/4-0026`, I²C bus **4**, address **0x26**.
- Driver **`ili2117`** bound under `/sys/bus/i2c/drivers/ili2117`; device-tree compatible **`ilitek,ili2117`**, state `okay`.
- IRQ threaded handler `irq/63-ili2117`; input event node `/dev/input/event0`.
- Current evdev bitmaps: `ev=b`, `key=400 0 0 0 0 0 0 0 0 0 0` and `abs=2608000 3`. For 32-bit ARM, these decode to:
  - `EV_SYN=0`, `EV_KEY=1`, `EV_ABS=3`;
  - `BTN_TOUCH=330`;
  - `ABS_X=0`, `ABS_Y=1`, `ABS_MT_SLOT=47`, `ABS_MT_POSITION_X=53`, `ABS_MT_POSITION_Y=54`, `ABS_MT_TRACKING_ID=57`.
- The GPIO key input is separate `/dev/input/event1`: its bitmap `key=100000 0 0 0` decodes to **`KEY_POWER=116`**. Both Akai and the separate power handler have that node open.

This establishes **Linux Type-B-style multi-touch tracking**. It **does not establish the maximum number of slots, the axis min/max/resolution, any rotation/calibration matrix, pressure support (not advertised), active finger count, or exactly what the upstream `ili210x` source implements**. The installed driver's binding is `ili2117`, which must not automatically be equated to the newer/other upstream `ili210x` source. Avoid firmware-update service or direct /dev/i2c bus access.

## Display/GPU: precise observed kernel properties

| Property | Value |
| --- | --- |
| DSI connector `card0-DSI-1` | connected, enabled, DPMS On, sole reported mode 800×1280 |
| `rockchipdrmfb` framebuffer | 800×3840 virtual, 32bpp, 3200 bytes per native row, `rotate=0` |
| GPU device-tree compatible | `rockchip,rk3288-mali` and `arm,mali-t760` |
| GPU driver | **Panfrost**, `/dev/dri/renderD128` |
| Backlight | `mipi-backlight`, current 106/255, active; no changes attempted |

Native 800×1280 is portrait; the real MPC One device is physically landscape. **Neither the exact DRM display transform nor touch rotation has been measured.** The virtual buffer's 3× native height is *consistent with* several buffers, not guaranteed triple buffering. DRM dumb-buffer pixel format / KMS atomicity and current DRM master remain untested.

## Highest-value next evidence / controlled experiments

1. **Metadata-only exact touch axes:** a separately authorized *non-grabbing*, non-event-reading Linux evdev `EVIOCGABS` ioctl can retrieve min/max/fuzz/flat/resolution for ABS_X, ABS_Y, ABS_MT_SLOT and ABS_MT_POSITION_X/Y. Opening an evdev descriptor is a small new interaction and is **not yet authorized by the existing read-only scripts**; implement and test it separately with explicit owner opt-in if appropriate.
2. **Watch-only touch sampling:** once the above proves safe and while factory GUI is idle, optionally read a short, nonexclusive event stream to empirically calibrate orientation and finger tracking. Never EVIOCGRAB, change kernel input mapping or record sensitive on-screen activities. This still needs separately scoped consent.
3. **Host-only offscreen test card and coordinate transforms:** Rust `mpfree-display` supports 1280×800 logical frame, 800×1280 native 90° alternatives, four distinct corner markers, and raw axis scaling only after range values are supplied. It produces files **on the workstation only** and does not acquire KMS/evdev.
4. **Actual factory app handoff remains BLOCKED** until independent original firmware/boot recovery, a separate supervised restoration timer, saved factory state and explicit owner consent. During future handoff keep `az01-power-button`, `az01-network-midi`, `az01-script-runner`, `mpfree-sshd` and storage helpers active; release only the main acvs group once we have demonstrated the exact stop behavior in a separately approved session.

This document contains only sanitized, non-identity-bearing observations, not raw logs or undocumented success claims.
