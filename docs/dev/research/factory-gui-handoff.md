# Factory process decomposition and reversible GUI handoff

**Status: READ-ONLY RESEARCH.** NO stop/start/restart/kill/disable/mask of factory services. NO DRM master, evdev/I2C device opens, modeset, reset or persistent file modifications. An actual takeover requires a separate owner-approved experiment with independent boot recovery already proven on this exact MPC One.

The user wants to replace the **factory application**, initially running alongside it and eventually taking over its audio/display/input resources. An MPFree Linux distribution is a long-term possibility; **replacing Linux is not the goal**.

## What is currently established

On original MPC One factory 3.6.0 / az0x 5.0.8 / kernel 6.6.95 PREEMPT_RT:

| Resource/service | Existing measured evidence | Consequence |
| --- | --- | --- |
| `acvs.service` | Main factory application launched by `/usr/bin/az01-launch-MPC`; real executable `/usr/bin/MPC`. Type=simple; Restart=always, 100ms delay; KillMode=control-group; 90s stop timeout. | Candidate for whole-app stop-and-restore **after recovery gates**. Intentional service stop is NOT auto-reversed by Restart=always. |
| Main MPC process | 38 *threads*, including main/JUCE message, Audio Processing on CPU3, AudioWorker0..3 across four CPUs, MIDI, file/other workers. | These are **one address space**, not 38 independently restorable processes. Do NOT target individual threads. |
| Device handles | Factory process opens `/dev/dri/card0`, `/dev/tty0`, `/dev/input/event0` (touch), `event1` (GPIO), ACVA playback/capture PCM, raw MIDI and ALSA sequencer. | Stopping it might release GUI, input and audio together; **not** a way to stop graphics only. |
| `az01-network-midi.service` | Separate simple service; Restart=always | Has independent process lifecycle, but dependencies require actual inspection. |
| `az01-script-runner.service` | Separate simple service; Restart=no; wanted/ordered by factory app | May be kept running; the Wants/After relation does not prove a stop cascade. |
| `az01-power-button.service` | Observed separate service | Should presumptively remain active; never disable shutdown handling without study. |
| `az01-unsupported-device-daemon.service` and `az01-usbsata-fixer.service` | Unit names observed; behavior not established | Determine executable and precise role before changes. |

A service `After=` constraint is only ordering; it is not equivalent to `Requires=` or `PartOf=`. Likewise, systemd `Restart=always` does not itself restart a service that has been deliberately stopped. Any eventual supervisor must explicitly restore it, wait for ownership reacquisition, and verify functionality.

## New read-only device-side investigation

Run after updating MPFree main on ML-tower:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash tools/mpc-one/collect-over-ssh.sh --process-map
bash tools/mpc-one/collect-over-ssh.sh --touchscreen
~~~

The first tool collects a bounded (180 PID max) **process tree** from `/proc`, without process command arguments or environments. It collects service unit names, precise systemd runtime/dependency/restart metadata for six known units, each factory thread's name, and **hardware fd symlink targets by process**, without opening hardware devices.

The touchscreen tool reads evdev capability bitmaps, I²C driver binding, compatible DT node, connector mode and framebuffer metadata. **No event stream is opened, no I²C bus transaction performed and no DRM framebuffer is acquired.**

Both run via the already-established direct Ethernet/IP/MAC + strict SSH host identity gate. Reports go only into locally ignored `research/`. **Redact volatile process IDs, host-identifying and potentially private service metadata before sharing or committing.** User project/sample contents, process argv/env, factory binaries and serial numbers are not collected.

## Research on other original MPC One application replacements

Granola/OpenMPC is publicly documented at https://github.com/nzimas/granola-openmpc . Its developer reports a successful manual **Akai → OpenMPC → Akai** transition without power cycling, including two automatic rollback tests, and an independent transient systemd safety timer armed **before** stopping the main application. It checks ACVA PCM ownership when returning. Their validation file is `openmpc/docs/validation-2026-08-14.md`.

This is valuable feasibility evidence, not a validated recipe for our device: the published experiment references firmware **3.9.1** and a newer **6.18** RT kernel, whereas the owner's is **3.6.0 / 6.6.95**. Their scripts also create persistent state, modify storage mount behavior and disable startup components in some modes. **Do not execute or directly transplant those scripts.**

An especially relevant observation from their validation: a `DeviceControlServer` was still present on a JUCE MIDI client after the main Akai application stopped. We must inspect our own factory process map to determine whether other daemons retain controller or MIDI ownership.

## Factory firmware update/recovery route: official documentation

Akai's own MPC Series Firmware Update Walkthrough documents a **manual power-on Update Mode** for the original MPC One using **FULL LEVEL + BROWSE + PAD BANK C/G** while powering on:
https://support.akaipro.com/en/support/solutions/articles/69000816160-akai-pro-mpc-series-firmware-update-walkthrough

This is useful to document before a supervised UI-handoff experiment. However, it is a **firmware updater entry mechanism**, not proven restoration from overwritten eMMC boot sectors, a damaged bootloader or completely unbootable Linux. Akai itself warns that interrupted firmware updates can sometimes be difficult or impossible to recover. We must **not trigger update mode, flash firmware, or alter boot partitions during read-only discovery**.

To qualify independent recovery later: identify available official firmware for the **exact original MPC One** and version, confirm updater/host-cable compatibility, maintain an externally stored full-storage/boot backup, and independently test a procedure for the relevant failure modes before altering factory disk or services. Do not infer that Rockchip generic MaskROM procedures or USB boot priority documented for other RK boards work on this product without direct evidence.

## Future owner-approved screen handoff stages (NOT implemented now)

**Stage 0: no device writes:** process and touchscreen survey; offscreen Rust test-card rendering on workstation; no DRM master, framebuffer, evdev, user project or factory service modifications.

**Stage 1: recovery proof:** external checksum-verified full eMMC/boot-area backup and independently tested factory firmware restoration **without relying on the factory OS booting**. Confirm recovery tools, boot process, media and return-to-stock procedures for this exact device. Save all open projects and factory state.

**Stage 2: independently supervised dry run:** simulate Acvs release, temporary DRM ownership, completion/crash, loss of SSH, missed heartbeat, restore failure, GUI/PCM failure and recovery-required status. The trusted supervisor must be outside MPFree's failure domain and arm its independent rollback timer **before** any service change. No silent disable/mask or persistent unit edits.

**Stage 3: temporary on-device transition, only after separate explicit consent:** with the factory quiescent, request a graceful stop of **only acvs.service**, respecting its 90-second stop timeout. Verify release of DRM and input handles; **abort** if unexpected owners remain. Render a very simple bounded test card directly to DSI-1 with a minimal DRM/KMS backend. Before restoring Akai, MPFree must close all DRM/input handles. The independent supervisor then starts the previously active stock service and verifies systemd, DRM/input ownership, ALSA playback/capture, and **user-confirmed touchscreen, pads, GUI and audio** functionality. If anything fails, report `RecoveryRequired`; do not pretend that an active PID guarantees a restored system.

Only after that experiment succeeds repeatedly can we consider running the real MPFree GUI during an exclusive session.

**Why not stop only the graphics threads?** There is no demonstrated stable boundary inside the one-process factory DAW. Its GUI, audio, I/O and worker threads share state. Trying to selectively kill/suspend them risks a permanently wedged app or lost user data. Investigate independently running services, but treat the factory application as a single unit for first handoff.

**Safety distinction:** Being able to restart a factory process is encouraging but **does not** prove recoverability from a kernel crash, broken kernel drivers, incomplete framebuffer restoration or unbootable eMMC. None of the current GitHub tools permit a real factory stop.
