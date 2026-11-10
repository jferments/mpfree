# Test-booting another firmware version by kexec (MPC One)

**Status (2026-11-10):** 3.10.0 has booted once on the reference unit, from the stick. Nothing in the MPC's storage changed: the boot maps before and after match, and 3.10.0 saw no MMC device. It reached its normal running state with no failed units, but had no Ethernet, so SSH never answered (see [First boot](#first-boot-2026-11-10)).

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
| 5 | `MPFREE_ACCEPT_KEXEC_BOOT=YES bash tools/mpc-one/collect-over-ssh.sh --kexec-boot` | **State-changing, RAM only on the MPC.** With the stick in the MPC's USB-A port, it runs [`host_kexec_boot.py`](../../tools/mpc-one/host_kexec_boot.py): checks on the workstation and on 3.6.0 (read-only), copies `zImage` and `kexec.dtb` to `/run`, runs `kexec --load`, and jumps (`systemctl kexec`) only after you type `BOOT 3.10`. Then it checks 3.10.0 over SSH. Details below |
| 6 | Power-cycle with the stick out, then `collect-over-ssh.sh --boot-map` | The MPC is back on 3.6.0. The boot map compares the boot-partition hashes and write counters with the run before the test |
| 7 | `python3 tools/mpc-one/read-stick-logs.py` | Read-only, with the stick in the workstation. It copies the log partition once with `sudo cat`, replays its journal on the copy, and writes every log of every test boot to `~/mpfree-work/stick-logs-*/report.txt`, with MACs and USB serials replaced |
| 8 | `python3 tools/mpc-one/offline-kexec-details.py` | Workstation only. When something fails in the test boot, it prints the Ethernet, display, backlight and reserved-memory nodes of both releases' board files, the running tree and `kexec.dtb` (MAC and serial values hidden), every difference between the two board files, and both kernels' build configs |

Each step runs through [`results.py`](../../tools/mpc-one/results.py), so every log and report of a round lands in `~/mpfree-results/current/` and comes back as one zip ([how](SURVEY_TOOLS.md#one-folder-and-one-zip-per-test-round)).

## The boot step

`host_kexec_boot.py` goes in this order and stops at the first problem:

1. **Workstation, read-only.** The newest `kexec-stick-*` folder matches its `SHA256SUMS`, its `kexec.dtb` hides both controllers (the same check the builder runs), and `cmdline.txt` has one `root=PARTUUID=` plus `ro rootwait=60 panic=10 sysctl.kernel.kexec_load_disabled=1`.
2. **3.6.0, read-only.** The running release is the one the build was made for, no kexec kernel is loaded, kexec is allowed, lockdown is off, this build's stick is on USB (3.6.0 mounts both of its partitions under `/media` when it is plugged in, and unmounts them when it shuts down), `/run` has 64 MiB free, and no unit or shutdown hook other than systemd's own reacts to kexec. It prints every line that mentions kexec. The one line it lets through is systemd 255.17's own `Conflicts=reboot.target kexec.target poweroff.target halt.target` in `systemd-fsck@.service`, which only stops a running file system check, as at any reboot. It records the eMMC write counters.
3. **RAM.** It copies `zImage` and `kexec.dtb` to `/run/mpfree-kexec` and compares their hashes. Undo: deleted on any later stop; `/run` is RAM.
4. **Kernel memory.** `kexec --load`, then `kexec_loaded` must read 1. Undo: `kexec --unload`.
5. **Your go.** You type `BOOT 3.10`. Anything else, or Ctrl-C, unloads the kernel, deletes the files and stops. 3.6.0 keeps running.
6. **The jump.** `systemctl kexec`: 3.6.0 shuts down cleanly and jumps to 3.10.0. If the jump itself fails, systemd reboots normally into 3.6.0.
7. **3.10.0, read-only.** It waits up to 300 s for SSH and accepts only the test build's own host key (`known_hosts`, alias `mpfree-kexec-test`). Then it checks: the release, that no `mmcblk` device exists and no driver is bound to either controller, that kexec is off, that `/` is the stick's partition 1 mounted read-only, and that `/data` is tmpfs. It also lists failed units, the masked units' states and every `mpfree-test: blocked` line.

The possible endings, and what to do after each:

| What it prints | Meaning | Next |
| --- | --- | --- |
| `RESULT: the test build is running from the stick, and it cannot see the eMMC or SD card.` | The test passed | Power-cycle with the stick out, then steps 6 and 7 |
| A `PROBLEM` line from the 3.10.0 checks | 3.10.0 is running, but something is not as designed | The same: power-cycle with the stick out, then steps 6 and 7 |
| `the MPC came back to 3.6` | The jump failed and systemd rebooted into 3.6.0 | Step 6, then step 7 |
| Nothing answered in time | 3.10.0 hung, or has no network | Power-cycle with the stick out, then steps 6 and 7. The stick's log partition shows how far it got |
| A stop before you type | Nothing was loaded, or the load was undone | Nothing to undo |

The stick's log service starts early in 3.10.0's boot (in `sysinit.target`) and keeps running through a switch to emergency mode. Every 10 seconds it saves `dmesg`, the journal, failed units, all units, the network state and the command line to the stick's partition 2, which is mounted `sync`. If the kernel cannot mount the root partition, it panics and reboots after 10 seconds (`panic=10`) and U-Boot boots 3.6.0 from the eMMC.

## First boot (2026-11-10)

- **Storage:** the boot maps before and after the test match: the same hashes for p1–p6 and the boot areas, and no writes. 3.10.0's kernel log shows no MMC device, and no blocked program was called. The power-cycle brought 3.6.0 back.
- **3.10.0 itself:** its startup finished 7.7 s after its kernel started, with no failed units and sshd listening on port 2222, and the log service wrote to the stick every 10 s.
- **Ethernet failed:** the MAC (`rk_gmac-dwmac ff290000.ethernet`) probed, but `mdio_bus stmmac-0: MDIO device at address 1 is missing`, and later `eth0: cannot attach to PHY (error: -ENODEV)`. Under 3.6.0 the RTL8211E answers at address 1. **Inferred, not verified:** 3.6.0 leaves the PHY held in reset when it shuts down for the jump, and 3.10.0 reads the PHY's ID before releasing the reset. `offline-kexec-details.py` shows how the board files wire the PHY reset, which settles it.
- **Screen:** the display driver came up (`fb0`) and a getty started on `tty1`, but nothing appeared. The kernel log has no framebuffer console line, so 3.10.0's kernel probably has none built in (inferred; its build config, step 8, settles it).
- **Reserved memory:** 3.10.0's kernel skipped U-Boot's `framebuffer` and `dma-unusable` regions as `invalid reg property`, after Akai's own fixup converted them, so DRM had no reserved framebuffer. That only affects the splash hand-over.

## Keep it private

The `kexec-stick-*` folder and the stick hold Akai's root filesystem. They also hold a test SSH host key, your public key, and, in `kexec.dtb`, the unit's serial number and MAC. Never commit or share them. The builder's `report.txt` names the changes only, and is fine to share.
