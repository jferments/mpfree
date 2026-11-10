# Survey tools: inspecting the MPC from a workstation

All device surveys run from the **workstation**, through one wrapper: [`tools/mpc-one/collect-over-ssh.sh`](../../tools/mpc-one/collect-over-ssh.sh). It streams a reviewed, repository-local script to the MPC over SSH and saves the report on the workstation. It does not accept arbitrary scripts or remote commands. Set up access first with [developer access](developer-access.md).

## Before every run, the wrapper checks

- the expected address on the expected network interface, and that the route to the MPC goes through it;
- that the MPC answers a direct ping and has the expected MAC address;
- the SSH host key (strict checking, batch mode, no agent forwarding).

The defaults match the tested reference lab (`eno1np0`, `10.42.0.1/24` → `10.42.0.141:2222`, the dedicated developer key). Anyone else must set their own verified values through the environment variables at the top of the script: `MPC_IP`, `MPC_IFACE`, `MPC_HOST_IP_CIDR`, `MPC_EXPECT_MAC`, `MPC_PORT`, `MPC_USER` and `MPC_KEY`.

## Modes

Run from an up-to-date checkout of `main`:

~~~sh
bash tools/mpc-one/collect-over-ssh.sh            # baseline inventory
bash tools/mpc-one/collect-over-ssh.sh --detail   # any mode from the tables below
~~~

**Read-only surveys.** These read procfs, sysfs and config files only, except `--boot-map`, which also reads the raw boot partitions with `dd` to hash them (reads only, no writes). `--kexec-preflight` also asks `kexec` and `systemctl` for their version and help text; it never loads a kernel. `--ethernet` also reads debugfs when it is already mounted, and asks `ethtool` (if installed) for its query output only. They open no audio, MIDI or display devices and change no services.

| Mode | Script | What it reports |
|---|---|---|
| *(none)* | [`read-only-inventory.sh`](../../tools/mpc-one/read-only-inventory.sh) | Baseline hardware and software inventory |
| `--detail` | [`read-only-detail.sh`](../../tools/mpc-one/read-only-detail.sh) | Services and resource ownership |
| `--runtime` | [`read-only-runtime.sh`](../../tools/mpc-one/read-only-runtime.sh) | Process threads and ALSA PCM state |
| `--io` | [`read-only-io.sh`](../../tools/mpc-one/read-only-io.sh) | Ethernet link/PHY and USB host/gadget capabilities |
| `--abi` | [`read-only-abi.sh`](../../tools/mpc-one/read-only-abi.sh) | Userspace ABI, loader and libraries |
| `--handoff-preflight` | [`read-only-handoff-preflight.sh`](../../tools/mpc-one/read-only-handoff-preflight.sh) | Factory service map |
| `--process-map` | [`read-only-process-map.sh`](../../tools/mpc-one/read-only-process-map.sh) | Process/unit tree and device ownership |
| `--touchscreen` | [`read-only-touchscreen.sh`](../../tools/mpc-one/read-only-touchscreen.sh) | ILI2117, DRM and input metadata |
| `--rtthreads` | [`read-only-rtthreads.sh`](../../tools/mpc-one/read-only-rtthreads.sh) | Per-task RT policy, priority and affinity |
| `--capacity` | [`read-only-capacity.sh`](../../tools/mpc-one/read-only-capacity.sh) | Two-second CPU and memory sample |
| `--boot-map` | [`read-only-boot-map.sh`](../../tools/mpc-one/read-only-boot-map.sh) | Boot chain, U-Boot environment (identifiers redacted), boot-partition SHA-256 hashes, eMMC write counters, kexec support, update-path metadata |
| `--kexec-preflight` | [`read-only-kexec-preflight.sh`](../../tools/mpc-one/read-only-kexec-preflight.sh) | What a kexec test boot needs: the kexec program's version and options, kexec state, memory map, CPU hotplug, systemd, RAM folders, and device-tree names and sizes. The wrapper also copies `/sys/firmware/fdt` to `~/mpfree-work/kexec-preflight-<UTC>-*/` (private: it holds the serial number and MAC) and compares it, without values, with Akai's device trees from `offline-rootfs-survey.sh` |
| `--ethernet` | [`read-only-ethernet.sh`](../../tools/mpc-one/read-only-ethernet.sh) | The Ethernet interfaces, MDIO bus and PHY, the regulator feeding the PHY, the PHY's reset and interrupt pins, the Ethernet clocks and pins from debugfs (only if already mounted; never mounts it), the kernel's Ethernet messages, and `ethtool` queries if it is installed. MACs are replaced. The kexec test build runs the same script and keeps its output on the stick ([kexec test boot](../hardware/kexec-test-boot.md)) |
| `--audio-midi` | [`read-only-audio-midi.sh`](../../tools/mpc-one/read-only-audio-midi.sh) | ALSA, device-tree audio nodes, IRQs, RT limits, cpufreq and USB MIDI descriptors |
| `--midi-activity` | [`read-only-midi-counters.sh`](../../tools/mpc-one/read-only-midi-counters.sh) | Interactive: RawMIDI counter changes while you press pads, knobs and buttons |
| `--ram-smoke-env` | [`read-only-ram-smoke-env.sh`](../../tools/mpc-one/read-only-ram-smoke-env.sh) | Utilities available for RAM-only tests |

**Milestone preflights and RAM-only runs.** Each `-check` mode is read-only. The matching run mode copies a checksummed binary to `/run` (RAM), runs it under a watchdog and removes it. Run modes require explicit opt-in environment variables; the wrapper prints the exact command if they are missing. See each milestone's doc for the procedure.

| Modes | Milestone |
|---|---|
| `--ram-smoke-check`, `--ram-smoke` | [M5](../milestones/m05-ram-only-rust-smoke.md) |
| `--headless-check`, `--headless-smoke` | [M6](../milestones/m06-headless-null-audio.md) |
| `--session-check`, `--session-smoke` | [M7](../milestones/m07-on-device-acceptance.md) |
| `--touch-axes-check`, `--touch-axes` | [M10](../milestones/m10-guarded-touch-axes.md) |

**Test boot of another firmware version.** `--kexec-boot` boots the test build on the USB stick once by kexec. It changes only RAM on the MPC, jumps only after you type `BOOT 3.10`, and requires `MPFREE_ACCEPT_KEXEC_BOOT=YES`. After it, [`read-stick-logs.py`](../../tools/mpc-one/read-stick-logs.py) copies the test build's logs off the stick, read-only, on the workstation, and [`offline-kexec-details.py`](../../tools/mpc-one/offline-kexec-details.py) prints the device-tree nodes and kernel configs behind the build when something fails in it. See [the kexec test boot](../hardware/kexec-test-boot.md).

**Passive capture.** `--midi-capture` reads the kernel's usbmon interface for the controller's USB bus while you use the controller. It is interactive, stops itself within 240 s and requires `MPFREE_ACCEPT_USBMON=YES`. See [audio and MIDI diagnostics](audio-midi-diagnostics.md).

**Active test.** `--netbench` runs [`network-bench.py`](../../tools/mpc-one/network-bench.py) on the workstation: 20 pings and a 4 MiB SSH transfer in each direction. It writes no files on the MPC but loads its CPU and network briefly, so only run it while the MPC is idle.

## Reports

Reports go to the local `research/` folder, which git ignores. They can contain device identifiers and proprietary paths. Publish only reviewed, redacted findings under `docs/hardware/`, and never commit raw logs, keys, firmware, sample content or images.

## One folder and one zip per test round

Run every step of a test round through [`results.py`](../../tools/mpc-one/results.py), so all of its output lands in one folder, `~/mpfree-results/current/`, and comes back as one zip:

```bash
python3 tools/mpc-one/results.py run boot-map -- bash tools/mpc-one/collect-over-ssh.sh --boot-map
python3 tools/mpc-one/results.py zip      # prints the one zip to upload
```

`run` keeps the terminal, so you can still type answers and your sudo password (which is not echoed, so not recorded). It saves everything the step prints to `NN-<name>.log`, ending with the exit code, and copies any report the step writes: a `report.txt` under `~/mpfree-work`, or a report in `research/`. `zip` adds `MANIFEST.txt` (the clone's commit, Python, kernel and a SHA-256 of each file), packs the folder into `~/mpfree-results/mpfree-results-<UTC>.zip`, and renames `current/` so the next round starts empty. Only logs and text reports are collected, never images, device trees, keys or backups. The reports can still quote Akai's files, so upload the zip to the project only.

## Other tools

- [`tools/collect-hardware-info.sh`](../../tools/collect-hardware-info.sh): the original lightweight survey, run by hand on the device.
- [`tools/mpc-one/gpu-qualification/`](../../tools/mpc-one/gpu-qualification/README.md): the offscreen EGL/GLES qualification kit.
- [`tools/mpc-one/host_audio_packet.py`](../../tools/mpc-one/host_audio_packet.py): host-only network-audio packet model, tested offline.
