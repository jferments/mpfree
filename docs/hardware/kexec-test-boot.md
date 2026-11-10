# Test-booting another firmware version by kexec (MPC One)

**Status (2026-11-10):** the tools that prepare the test are written and tested on fakes. The owner has run the read-only steps. **The boot step is not written yet, and nothing has been booted.**

**Goal:** boot Akai's 3.10.0 once on the reference unit without installing it, in a way where nothing in the MPC's own storage can change. 3.6.0 starts kexec from its running system, and 3.10.0's root filesystem comes from a USB stick.

Background: [Reading Akai's updater offline](akai-updater-offline.md) (what 3.10.0 contains and why its updater is dangerous) and the [boot map](MPC_ONE_3_6_0_BOOT_MAP_20261109.md) (the hashes and write counters the undo step compares).

## Why the eMMC stays untouched

| Layer | What it does | Where |
| --- | --- | --- |
| 1. The controllers are hidden | 3.10.0's device tree marks the eMMC controller (`ff0f0000`) and the SD card controller (`ff0c0000`) `disabled`. No driver binds, so no block device exists, and no program can open or write either one. **This is the main guarantee.** | `offline-dtb.py kexec-dtb` |
| 2. No second kexec | `sysctl.kernel.kexec_load_disabled=1` on the kernel command line, again in `sysctl.d`, and `kexec` replaced by a stub. Akai's updater reaches the eMMC only by kexec'ing its own kernel, and this closes that path. | `cmdline.txt`, the stick |
| 3. Nothing that flashes runs | Masked: the MPC app, the touch, XMOS and USB-SATA firmware updaters, the network update server, the USB script runner, the `/data` and content formatters, the firewall, and the eMMC content generator. Stubbed: `az01-update`, `az01config`, `midifirmup`, `dfu-util`, `touch-fw-upd`, `xmosdfu`, `fw_setenv` and others. | the stick |
| 4. `/data` is RAM | `data.mount` is replaced by a tmpfs, so the `/etc` and `/var` overlays live in RAM | the stick |
| Undo | Power-cycle. U-Boot boots 3.6.0 from the eMMC as before. A `--boot-map` run afterwards compares the boot-partition hashes and write counters. | |

These can still change: the USB stick (its root partition is mounted read-only, and its log partition is written), the real-time clock, and volatile PMIC and clock registers, which reset at power-off. A normal boot changes those too.

Some points here are inferences, not yet checked on the unit:
- LoadPin (`CONFIG_SECURITY_LOADPIN_ENFORCE=y` in 3.10.0) should allow modules and firmware from the stick, because the stick is the first filesystem they load from.
- The DHCP server should hand 3.10.0 the same address, because the device tree carries the same MAC.
- 3.10.0's `sshd` should accept the configuration that worked on 3.6.0.

## Which device tree

- U-Boot loads `rk3288-az01-acva.dtb` on this unit, not `-acva-c`. The running tree is plain `rockchip,rk3288`, and its CPU table stops at 1608 MHz; the `-c` file adds 1.7 and 1.8 GHz steps. The builder takes the file whose compatible list equals the running tree's.
- At boot, U-Boot adds several things to the tree:
  - the serial number
  - `inmusic,az01-pcb-rev` and `inmusic,az01-pcb-suffix`
  - the Ethernet MAC
  - `/chosen/bootargs`
  - one memory reservation: the tree's own location, `0xd000` bytes, the size of the tree
- `kexec-dtb` copies all of these into 3.10.0's file except `bootargs`, which kexec sets, and the reservation.
- It drops `/chosen/inmusic,acvs-emmc-content`, which tells Akai's units that the content partition is on the eMMC.

## Tools, in order

| Step | Command | Touches |
| --- | --- | --- |
| 1 | `bash tools/mpc-one/offline-rootfs-survey.sh` and `--from-backup` | Workstation only. It copies each version's `/boot` to `~/mpfree-work/boot-files/<release>/` |
| 2 | `bash tools/mpc-one/collect-over-ssh.sh --kexec-preflight` | Read-only on the MPC. It copies the running tree to `~/mpfree-work/kexec-preflight-*/live.dtb` |
| 3 | `python3 tools/mpc-one/offline-kexec-stick.py` | Workstation only, no devices. It changes a copy of the rootfs and writes a new folder `~/mpfree-work/kexec-stick-<UTC>-*/`: `stick.img` (GPT, with the rootfs as partition 1 and a 32 MiB log partition as partition 2), `zImage`, `kexec.dtb`, `cmdline.txt` and `known_hosts` |
| 4 | `python3 tools/mpc-one/write-test-stick.py` | **Erases one USB stick.** It writes only to the one removable whole disk on USB of 256 GiB or less, with nothing mounted, no swap and no holders, and only after you type `WRITE` |
| 5 | The boot step (not written yet) | It copies `zImage` and `kexec.dtb` to RAM on the MPC, loads them with `kexec -l`, and reboots into 3.10.0 only after a typed confirmation |

Each step runs through [`results.py`](../../tools/mpc-one/results.py), so every log and report of a round lands in `~/mpfree-results/current/` and comes back as one zip ([how](SURVEY_TOOLS.md#one-folder-and-one-zip-per-test-round)).

When 3.10.0 doesn't come up on the network, power-cycle the MPC and read the stick's log partition on the workstation. It holds `dmesg`, the journal, failed units and the network state, written every 10 seconds.

## Keep it private

The `kexec-stick-*` folder and the stick hold Akai's root filesystem. They also hold a test SSH host key, your public key, and, in `kexec.dtb`, the unit's serial number and MAC. Never commit or share them. The builder's `report.txt` names the changes only, and is fine to share.
