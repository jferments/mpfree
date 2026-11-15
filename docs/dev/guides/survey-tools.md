# Survey tools: inspecting the MPC from a workstation

All device surveys run from the **workstation**, through one wrapper: [`tools/mpc-one/collect-over-ssh.sh`](../../../tools/mpc-one/collect-over-ssh.sh). It streams a reviewed, repository-local script to the MPC over SSH and saves the report on the workstation. It does not accept arbitrary scripts or remote commands. Set up access first with [developer access](developer-access.md).

## Before every run, the wrapper checks

- the expected address on the expected network interface, and that the route to the MPC goes through it;
- that the MPC answers a direct ping and has the expected MAC address;
- the SSH host key (strict checking, batch mode, no agent forwarding). On an installed MPC OS 3.10.1, record developer mode's host key once with `--devmode-trust` (below).

The defaults match the tested reference lab (`eno1np0`, `10.42.0.1/24` → `10.42.0.141`, the dedicated developer key). Two SSH servers answer there, on two ports:

- **Akai's system on `MPC_PORT`, default 22:** the SSH server developer mode starts on an installed MPC OS 3.10.1 (since 2026-11-15). A unit still on 3.6.0 with our own SSH ([MOD-001](../hardware/archive/2026-11/device-modifications.md)) answers on 2222: set `MPC_PORT=2222`.
- **The test build, the USB stick's own system, on `MPC_TEST_PORT`, default 2222:** `--app` (and `--kexec-boot` after its jump, and `--devmode-check`), with the host key the stick was made with. On an installed MPC OS 3.10.1 the boot hook starts it: with the hook installed (`--hook-install`, below), switch the MPC on with the stick in.

Anyone else must set their own verified values through the environment variables at the top of the script: `MPC_IP`, `MPC_IFACE`, `MPC_HOST_IP_CIDR`, `MPC_EXPECT_MAC`, `MPC_PORT`, `MPC_TEST_PORT`, `MPC_USER` and `MPC_KEY`.


## Modes

Run from an up-to-date checkout of `main`:

~~~sh
bash tools/mpc-one/collect-over-ssh.sh            # baseline inventory
bash tools/mpc-one/collect-over-ssh.sh --detail   # any mode from the tables below
~~~

**Read-only surveys.** These read procfs, sysfs and config files only, except `--boot-map`, which also reads the raw boot partitions with `dd` to hash them (reads only, no writes). `--kexec-preflight` also asks `kexec` and `systemctl` for their version and help text; it never loads a kernel. `--ethernet` also reads debugfs when it is already mounted, and asks `ethtool` (if installed) for its query output only. They open no audio, MIDI or display devices and change no services.

| Mode | Script | What it reports |
|---|---|---|
| *(none)* | [`read-only-inventory.sh`](../../../tools/mpc-one/read-only-inventory.sh) | Baseline hardware and software inventory |
| `--detail` | [`read-only-detail.sh`](../../../tools/mpc-one/read-only-detail.sh) | Services and resource ownership |
| `--runtime` | [`read-only-runtime.sh`](../../../tools/mpc-one/read-only-runtime.sh) | Process threads and ALSA PCM state |
| `--io` | [`read-only-io.sh`](../../../tools/mpc-one/read-only-io.sh) | Ethernet link/PHY and USB host/gadget capabilities |
| `--abi` | [`read-only-abi.sh`](../../../tools/mpc-one/read-only-abi.sh) | Userspace ABI, loader and libraries |
| `--handoff-preflight` | [`read-only-handoff-preflight.sh`](../../../tools/mpc-one/read-only-handoff-preflight.sh) | Factory service map |
| `--process-map` | [`read-only-process-map.sh`](../../../tools/mpc-one/read-only-process-map.sh) | Process/unit tree and device ownership |
| `--touchscreen` | [`read-only-touchscreen.sh`](../../../tools/mpc-one/read-only-touchscreen.sh) | ILI2117, DRM and input metadata |
| `--rtthreads` | [`read-only-rtthreads.sh`](../../../tools/mpc-one/read-only-rtthreads.sh) | Per-task RT policy, priority and affinity |
| `--plugin-host` | [`read-only-plugin-host.sh`](../../../tools/mpc-one/read-only-plugin-host.sh) | For the [MPC plugin SDK work](../research/mpc-plugin-sdk.md), on MPC OS 3.10.1: firmware, SSH servers and listening ports, active `sshd_config` lines, whether `mpfree-sshd` survived; `acvs.service` user, limits and sandbox properties; the app process's user, capabilities, `NoNewPrivs`, seccomp, memory limits, threads (CPU, policy, RT priority) and plugin `.so` names it has loaded; names in each drive's `VST2 Plugins` and `Synths` folders; settings lines that mention plugins or enhancements; security modules, signed-filesystem units and mounts; free memory, `/dev/shm`; the last app log lines about plugins |
| `--capacity` | [`read-only-capacity.sh`](../../../tools/mpc-one/read-only-capacity.sh) | Two-second CPU and memory sample |
| `--boot-map` | [`read-only-boot-map.sh`](../../../tools/mpc-one/read-only-boot-map.sh) | Boot chain, U-Boot environment (identifiers redacted), boot-partition SHA-256 hashes, eMMC write counters, kexec support, update-path metadata |
| `--boot-menu-survey` | [`read-only-boot-menu.sh`](../../../tools/mpc-one/read-only-boot-menu.sh) | What the boot menu (`rust/mpfree-splash --menu`) needs: framebuffer sysfs, the controller's ALSA card and RawMIDI subdevices, which processes hold the display, MIDI and input devices (`readlink` of `/proc/*/fd` only), `acvs.service` and `az01-script-runner.service` properties, unit text and start times, how sticks are mounted, kexec state and free `/run` space |
| `--kexec-preflight` | [`read-only-kexec-preflight.sh`](../../../tools/mpc-one/read-only-kexec-preflight.sh) | What a kexec test boot needs: the kexec program's version and options, kexec state, memory map, CPU hotplug, systemd, RAM folders, and device-tree names and sizes. The wrapper also copies `/sys/firmware/fdt` to `~/mpfree-work/kexec-preflight-<UTC>-*/` (private: it holds the serial number and MAC) and compares it, without values, with Akai's device trees from `offline-rootfs-survey.sh` |
| `--ethernet` | [`read-only-ethernet.sh`](../../../tools/mpc-one/read-only-ethernet.sh) | The Ethernet interfaces, MDIO bus and PHY, the regulator feeding the PHY, the PHY's reset and interrupt pins, the Ethernet clocks and pins from debugfs (only if already mounted; never mounts it), the kernel's Ethernet messages, and `ethtool` queries if it is installed. MACs are replaced. The kexec test build runs the same script and keeps its output on the stick ([kexec test boot](../hardware/archive/2026-11/kexec-test-boot.md)) |
| `--audio-midi` | [`read-only-audio-midi.sh`](../../../tools/mpc-one/read-only-audio-midi.sh) | ALSA, device-tree audio nodes, IRQs, RT limits, cpufreq and USB MIDI descriptors |
| `--midi-activity` | [`read-only-midi-counters.sh`](../../../tools/mpc-one/read-only-midi-counters.sh) | Interactive: RawMIDI counter changes while you press pads, knobs and buttons |
| `--ram-smoke-env` | [`read-only-ram-smoke-env.sh`](../../../tools/mpc-one/read-only-ram-smoke-env.sh) | Utilities available for RAM-only tests |
| `--system-survey` | [`read-only-system-310.sh`](../../../tools/mpc-one/read-only-system-310.sh) | Facts about an installed MPC OS 3.10.1: versions, CPU clocks and idle states, memory, kexec and kernel security (lockdown, LoadPin), network interfaces and listening ports, sound and MIDI (cards, sequencer clients, `aconnect -l`, `amidi -l`), input devices, drives and their top-level names, screen and GPU devices, controller scripts, and the thread count of Akai's app. The CPU serial number is left out |

**Milestone preflights and RAM-only runs.** Each `-check` mode is read-only. The matching run mode copies a checksummed binary to `/run` (RAM), runs it under a watchdog and removes it. Run modes require explicit opt-in environment variables; the wrapper prints the exact command if they are missing. See each milestone's doc for the procedure.

| Modes | Milestone |
|---|---|
| `--ram-smoke-check`, `--ram-smoke` | [M5](../milestones/m05-ram-only-rust-smoke.md) |
| `--headless-check`, `--headless-smoke` | [M6](../milestones/m06-headless-null-audio.md) |
| `--session-check`, `--session-smoke` | [M7](../milestones/m07-on-device-acceptance.md) |
| `--touch-axes-check`, `--touch-axes` | [M10](../milestones/m10-guarded-touch-axes.md) |

**Test boot by kexec (3.6.0 only).** `--kexec-boot` boots the test build on the USB stick once by kexec. It changes only RAM on the MPC, jumps only after you type `BOOT 3.10`, and requires `MPFREE_ACCEPT_KEXEC_BOOT=YES`. On an installed MPC OS 3.10.1, LoadPin refuses kexec, so its checks stop before anything is loaded: there the boot hook (below) starts the test build at power-on. After either start, [`read-stick-logs.py`](../../../tools/mpc-one/read-stick-logs.py) copies the test build's logs off the stick, read-only, on the workstation, and [`offline-kexec-details.py`](../../../tools/mpc-one/offline-kexec-details.py) prints the device-tree nodes and kernel configs behind the build when something fails in it. See [the kexec test boot](../hardware/archive/2026-11/kexec-test-boot.md).

**mpfree on the test build.** `--app` runs Phase 1 on the running test build: our own program draws on the screen, reads touches and pads, lights the pads, replays Akai's controller setup, plays drums from the pads, runs the pad player (MPFree's first GUI: factory kits picked with the wheel, level on Q-Link 1, a Debug page for MIDI) and a quiet click, one step per Enter. It connects only to the test build (by its host key), copies the program to `/run`, changes RAM only, and requires `MPFREE_ACCEPT_APP=YES`. `MPFREE_APP_STEPS` picks the steps. The drum kits come from [`offline-factory-kit.py`](../../../tools/mpc-one/offline-factory-kit.py), which runs on the workstation only: it reads the content partition of the eMMC backup with `debugfs`, read-only, and writes up to 8 kits (`--kits N`) into one `kits.bin` under `~/mpfree-work` (Akai's content: never commit or share it). See [mpfree](mpfree-app.md).

**Akai's developer mode on an installed MPC OS 3.10.1.** [`host_devmode_login.py`](../../../tools/mpc-one/host_devmode_login.py) runs these four modes, on `MPC_PORT` (22). As Akai ships developer mode, its SSH server lets root in with no key and no password, so anyone on the MPC's network can get a root shell while it is on.

- `--devmode-trust`, **read-only on the MPC:** reads the server's ed25519 host key and prints its fingerprint. Only after a typed `TRUST`, it adds that one line to `~/.ssh/known_hosts`, the file every other mode checks against. It never replaces a different key already recorded: it prints the `ssh-keygen -R` line for that instead.
- `--devmode-status`, **read-only:**
    - where `/` and `/data` come from, the `/etc` overlay, and the server's settings as it reads them (`sshd -T`);
    - `allow-access.conf`, how many keys `authorized_keys` holds, the kind of root's password field (never its value) and the listening TCP ports;
    - the host key's fingerprint and date, and the time since the MPC started;
    - then a login with no key and one with your key, each running only `id -u`.
- `--devmode-lock`, **state-changing on `/data`, after a typed `LOCK`:** Akai's own lock-down.
    - It copies both files to `/data/ssh/mpfree-lock-backup`, adds your public key to `/data/ssh/authorized_keys`, empties `allow-access.conf` and reloads the server.
    - It keeps one connection open throughout. If the login with no key is not refused afterwards, or your key does not get in, or the server still accepts empty passwords, it puts both files back over that connection.
- `--devmode-unlock`, **state-changing on `/data`, after a typed `UNLOCK`:** the undo of the lock.

See [the 3.10.1 qualification runbook](../hardware/firmware-3.10.1-qualification.md) for when to run them and the undo by hand from the developer panel's terminal.

**Akai's developer mode on the test build.** `--devmode-check` runs on a 3.10.1 test build made with `offline-kexec-stick.py --akai-app`, after developer mode has been switched on in Akai's app. It is read-only: over port 2222 it reads what developer mode changed (the `/etc` overlay, `/data/ssh`, `sshd -T`, the host key, listening ports, the kind of root's password field, the journal), then logs in on port 22 from the workstation with no key and with your key and runs only `id -u`, accepting only the host key it read over port 2222. Only after a typed `LOCK` it does Akai's own lock-down to your key, in RAM only. [`write-autorun-drive.py`](../../../tools/mpc-one/write-autorun-drive.py) puts the harmless `autorun.sh` for the same test on a spare USB drive, on the workstation; when it finds no usable drive, it lists the USB drives it sees with nothing mounted and says what to do (it never formats or mounts anything itself). See [the 3.10.1 stick test](../hardware/firmware-3.10.1-stick-test.md#phase-3-developer-mode-on-the-stick).

**Boot-hook canary (installed 3.10.1, port 22 only).** Like every mode on port 22, it needs developer mode's host key recorded first (`--devmode-trust`). `--hook-canary` writes three small files to the MPC's data partition: a udev rule and a modprobe line in Akai's writable `/etc` layer and the script they run, which only log at power-on (when udev loads the rule, when a USB stick appears, when the Wi-Fi module loads) and whether Akai's app has started, and start a short service that logs too. It needs `MPFREE_ACCEPT_HOOK_CANARY=YES`. `--hook-canary-report` reads the result (read-only) and `--hook-canary-remove` is the undo. See [starting MPFree from the stick](../hardware/stick-autoboot.md#the-canary-steps-1-and-2).

**Boot hook (installed 3.10.1, port 22 only).** `--hook-install` writes MPFree's boot hook in the canary's places (and deletes the canary): from the next power-on, with MPFree's stick in, a modprobe line (and a udev rule as the backup) starts a short service that Akai's app and its firmware updaters wait for; it mounts the stick's system read-only at `/run/nextroot`, checks it against `/boot/mpfree-softreboot` (it must switch the eMMC and SD card off first) and soft-reboots into it on Akai's running kernel. Without the stick, or on any problem, Akai's system starts as usual. With the stick in at install, it also checks the stick (read-only). It needs `MPFREE_ACCEPT_HOOK=YES`. `--hook-report` reads what the hook did at this power-on (read-only) and `--hook-remove` is the undo. See [starting MPFree from the stick](../hardware/stick-autoboot.md#the-soft-reboot-hook-step-4).

**Akai's app at startup.** `--app-start-capture` runs on stock 3.6 only. It records the controller's USB traffic (usbmon, passive) while systemd restarts Akai's app once, so the report shows what the app sends to the controller as it starts. The restart is the one state change: it needs `MPFREE_ACCEPT_APP_RESTART=YES` and a typed `RESTART`, and checks afterwards that the app runs again and the controller is the same USB device. See [mpfree](mpfree-app.md#how-round-2-captured-akais-startup).

**Passive capture.** `--midi-capture` reads the kernel's usbmon interface for the controller's USB bus while you use the controller. It is interactive, stops itself within 240 s and requires `MPFREE_ACCEPT_USBMON=YES`. See [audio and MIDI diagnostics](audio-midi-diagnostics.md).

**Active test.** `--netbench` runs [`network-bench.py`](../../../tools/mpc-one/network-bench.py) on the workstation: 20 pings and a 4 MiB SSH transfer in each direction. It writes no files on the MPC but loads its CPU and network briefly, so only run it while the MPC is idle.

## Reports

Reports go to the local `research/` folder, which git ignores. They can contain device identifiers and proprietary paths. Publish only reviewed, redacted findings under `docs/dev/hardware/`, and never commit raw logs, keys, firmware, sample content or images.

## One folder and one zip per test round

Run every step of a test round through [`results.py`](../../../tools/mpc-one/results.py), so all of its output lands in one folder, `~/mpfree-results/current/`, and comes back as one zip:

```bash
python3 tools/mpc-one/results.py run boot-map -- bash tools/mpc-one/collect-over-ssh.sh --boot-map
python3 tools/mpc-one/results.py zip      # prints the one zip to upload
```

`run` keeps the terminal, so you can still type answers and your sudo password (which is not echoed, so not recorded). It saves everything the step prints to `NN-<name>.log`, ending with the exit code, and copies any report the step writes: a `report.txt` under `~/mpfree-work`, or a report in `research/`. `zip` adds `MANIFEST.txt` (the clone's commit, Python, kernel and a SHA-256 of each file), packs the folder into `~/mpfree-results/mpfree-results-<UTC>.zip`, and renames `current/` so the next round starts empty. Only logs and text reports are collected, never images, device trees, keys or backups. The reports can still quote Akai's files, so upload the zip to the project only.

## Other tools

- [`tools/collect-hardware-info.sh`](../../../tools/collect-hardware-info.sh): the original lightweight survey, run by hand on the device.
- [`tools/mpc-one/gpu-qualification/`](../../../tools/mpc-one/gpu-qualification/README.md): the offscreen EGL/GLES qualification kit.
- [`tools/mpc-one/host_audio_packet.py`](../../../tools/mpc-one/host_audio_packet.py): host-only network-audio packet model, tested offline.
