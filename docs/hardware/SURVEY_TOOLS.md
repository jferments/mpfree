# Survey tools: inspecting the MPC from a workstation

All device surveys run from the **workstation**, through one wrapper: [`tools/mpc-one/collect-over-ssh.sh`](../../tools/mpc-one/collect-over-ssh.sh). It streams a reviewed, repository-local script to the MPC over SSH and saves the report on the workstation. It does not accept arbitrary scripts or remote commands. Set up access first with [developer access](DEVELOPER_ACCESS.md).

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

**Read-only surveys.** These read procfs, sysfs and config files only, except `--boot-map`, which also reads the raw boot partitions with `dd` to hash them (reads only, no writes). They open no audio, MIDI or display devices and change no services.

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
| `--ram-smoke-env` | [`read-only-ram-smoke-env.sh`](../../tools/mpc-one/read-only-ram-smoke-env.sh) | Utilities available for RAM-only tests |

**Milestone preflights and RAM-only runs.** Each `-check` mode is read-only. The matching run mode copies a checksummed binary to `/run` (RAM), runs it under a watchdog and removes it. Run modes require explicit opt-in environment variables; the wrapper prints the exact command if they are missing. See each milestone's doc for the procedure.

| Modes | Milestone |
|---|---|
| `--ram-smoke-check`, `--ram-smoke` | [M5](../engineering/ARMV7_VOLATILE_SMOKE.md) |
| `--headless-check`, `--headless-smoke` | [M6](../engineering/M6_HEADLESS_NULL_AUDIO.md) |
| `--session-check`, `--session-smoke` | [M7](../engineering/M7_ARM_NULL_TIMING_ACCEPTANCE.md) |
| `--touch-axes-check`, `--touch-axes` | [M10](../engineering/M10_GUARDED_TOUCH_AXES.md) |

**Active test.** `--netbench` runs [`network-bench.py`](../../tools/mpc-one/network-bench.py) on the workstation: 20 pings and a 4 MiB SSH transfer in each direction. It writes no files on the MPC but loads its CPU and network briefly, so only run it while the MPC is idle.

## Reports

Reports go to the local `research/` folder, which git ignores. They can contain device identifiers and proprietary paths. Publish only reviewed, redacted findings under `docs/hardware/`, and never commit raw logs, keys, firmware, sample content or images.

## Other tools

- [`tools/collect-hardware-info.sh`](../../tools/collect-hardware-info.sh): the original lightweight survey, run by hand on the device.
- [`tools/mpc-one/gpu-qualification/`](../../tools/mpc-one/gpu-qualification/README.md): the offscreen EGL/GLES qualification kit.
- [`tools/mpc-one/host_audio_packet.py`](../../tools/mpc-one/host_audio_packet.py): host-only network-audio packet model, tested offline.
