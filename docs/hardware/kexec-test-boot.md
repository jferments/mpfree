# Test-booting another firmware version by kexec (MPC One)

**Status (2026-11-10):** 3.10.0 has booted three times on the reference unit, from the stick. Nothing in the MPC's storage changed any time: the boot maps before and after match the first baseline, and 3.10.0 saw no MMC device. Since the second boot it shows its live log on the MPC's screen. On the third boot Ethernet and SSH worked, after the test build started `eth0` a second time (see [Third boot](#third-boot-2026-11-10)). **The test is finished** (owner's decision, 2026-11-10): the goal is met. The first-start failure of `eth0` is left open until a system of ours is started by kexec.

**Goal:** boot Akai's 3.10.0 once on the reference unit without installing it, in a way where nothing in the MPC's own storage can change. 3.6.0 starts kexec from its running system, and 3.10.0's root filesystem comes from a USB stick.

Background: [Reading Akai's updater offline](akai-updater-offline.md) (what 3.10.0 contains and why its updater is dangerous) and the [boot map](boot-map.md) (the hashes and write counters the undo step compares).

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
- It puts the Ethernet PHY's ID first in the PHY node's compatible list: `"ethernet-phy-id001c.c915"` (Realtek RTL8211E, as 3.6.0 reads it on this unit). Both releases' board files give the PHY (`/ethernet@ff290000/mdio/ethernet-phy@1`) its own reset line, `reset-gpios` on GPIO4 B0, active low, with Akai's 10 ms assert and 30 ms deassert times. The kernel drives that line low when it lets go of the PHY, as 3.6.0 does when it shuts down for the jump. Without an ID in the tree, the new kernel reads the ID from the PHY before it takes that line, so it finds nothing (the first boot's `MDIO device at address 1 is missing`). With the ID, it skips that read, takes the line, resets the PHY with Akai's times and only then talks to it. This is the kernel's documented way for PHYs whose ID can't be read at that point.

## The screen

Neither kernel has a text console on the screen (`CONFIG_FRAMEBUFFER_CONSOLE` is off in both), so the stick adds our own program, [`mpfree-screen-log`](../../rust/mpfree-screen-log). A service started early in the boot pipes `journalctl -f` into it, and it draws the newest lines on `/dev/fb0` about four times a second, with a title row and a status row (uptime and each network link's state). It reads the framebuffer's size, stride and depth from sysfs. It turns the text by the panel's `rotation` in the device tree (270 on the MPC One: the panel's left edge is at the top), which agrees with the M12 touch calibration. After the first frame, it writes `0,0` to the framebuffer's sysfs `pan` file. That makes the kernel's framebuffer emulation set its mode, which turns the panel on. It writes nothing else.

It is a static ARM executable, so it doesn't depend on 3.10.0's C library. Build it on the workstation before the builder:

```bash
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" \
cargo build --manifest-path rust/Cargo.toml --release --locked \
  --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log
```

The builder refuses anything but a static ARM EABI5 hard-float executable. CI builds it the same way and runs it under `qemu-arm` against a fake framebuffer. `--preview FILE.ppm` draws a sample frame on the workstation instead.

**Startup screen (optional).** With `--splash FILE`, the builder also puts our startup-screen program `mpfree-splash` (the galaxy, from `rust/mpfree-splash`) on the stick. A oneshot service draws it on `/dev/fb0` for 6 s, turned by the same panel rotation, and the log display starts when it ends (if it fails, the log simply starts). `--splash-font FILE` adds a `.ttf` or `.otf` font for its title; without one the title uses built-in letters. The program is built like the screen log (`-p mpfree-splash`) and checked the same way. It writes only the framebuffer and its sysfs `pan` file. The boot menu is not on the stick yet. Without Akai's app the controller sends its wheel and buttons on the Public port, which round 1 did not listen to (see [the controller protocol](controller-protocol.md#without-akais-app-hello-mpc-rounds-1-and-2-2026-11-10)).

**Pad player at power-on (optional).** With `--app FILE` (our [`mpfree-hello`](../../rust/mpfree-hello), built like the screen log), the stick starts the pad player instead of the log display: a loading page with a progress bar, then the app ([Hello MPC round 5](../guides/hello-mpc.md#round-5-straight-into-the-pad-player)). Its service, `mpfree-test-app`, starts early like the log display, after the startup screen when there is one, and uses its last frame as the loading page's backdrop. If the app stops with an error, `OnFailure=` starts the log display instead. The drum kits (`--kits FILE`, by default the newest `~/mpfree-work/factory-kit-*/kits.bin`) go on a third partition: a small ext4 file system without a journal that holds only `kits.bin`, which the app's start script mounts read-only. So the root filesystem copy needs no extra room, and partitions 1 and 2 stay as they were. The kits are Akai's sounds, so the stick and its build folder are private. Stock 3.6.0 mounts this partition under `/media` too while the stick is in the MPC, as it does the other two.

## Tools, in order

| Step | Command | Touches |
| --- | --- | --- |
| 1 | `bash tools/mpc-one/offline-rootfs-survey.sh` and `--from-backup` | Workstation only. It copies each version's `/boot` to `~/mpfree-work/boot-files/<release>/` |
| 2 | `bash tools/mpc-one/collect-over-ssh.sh --kexec-preflight` | Read-only on the MPC. It copies the running tree to `~/mpfree-work/kexec-preflight-*/live.dtb` |
| 3 | `python3 tools/mpc-one/offline-kexec-stick.py`, after building `mpfree-screen-log` ([the screen](#the-screen)) | Workstation only, no devices. It changes a copy of the rootfs and writes a new folder `~/mpfree-work/kexec-stick-<UTC>-*/`: `stick.img` (GPT, with the rootfs as partition 1, a 32 MiB log partition as partition 2 and, with `--app`, the drum kits as partition 3 and the saved settings as partition 4, 64 MiB at 1 GiB), `zImage`, `kexec.dtb`, `cmdline.txt` and `known_hosts` |
| 4 | `python3 tools/mpc-one/write-test-stick.py` | **Erases one USB stick.** It writes only to the one removable whole disk on USB of 256 GiB or less, with nothing mounted, no swap and no holders, and only after you type `WRITE`. Partition 4 fills the rest of the stick and keeps the saved settings ([below](#saved-settings-partition-4)) |
| 5 | `MPFREE_ACCEPT_KEXEC_BOOT=YES bash tools/mpc-one/collect-over-ssh.sh --kexec-boot` | **State-changing, RAM only on the MPC.** With the stick in the MPC's USB-A port, it runs [`host_kexec_boot.py`](../../tools/mpc-one/host_kexec_boot.py): checks on the workstation and on 3.6.0 (read-only), copies `zImage` and `kexec.dtb` to `/run`, runs `kexec --load`, and jumps (`systemctl kexec`) only after you type `BOOT 3.10`. Then it checks 3.10.0 over SSH. Details below |
| 6 | Power-cycle with the stick out, then `collect-over-ssh.sh --boot-map` | The MPC is back on 3.6.0. The boot map compares the boot-partition hashes and write counters with the run before the test |
| 7 | `python3 tools/mpc-one/read-stick-logs.py` | Read-only, with the stick in the workstation. It copies the log partition once with `sudo cat`, replays its journal on the copy, and writes every log of every test boot to `~/mpfree-work/stick-logs-*/report.txt`, with MACs and USB serials replaced |
| 8 | `python3 tools/mpc-one/offline-kexec-details.py` | Workstation only. When something fails in the test boot, it prints the Ethernet, display, backlight and reserved-memory nodes of both releases' board files, the running tree and `kexec.dtb` (MAC and serial values hidden), every difference between the two board files, and both kernels' build configs |
| 9 | `bash tools/mpc-one/collect-over-ssh.sh --ethernet` | Read-only on the MPC. Runs [`read-only-ethernet.sh`](../../tools/mpc-one/read-only-ethernet.sh) on 3.6.0: the interfaces, the MDIO bus and PHY, the PHY's reset and interrupt pins, the Ethernet clocks (debugfs, only if already mounted), and the kernel's Ethernet messages, with MACs replaced. The test build runs the same script and keeps its output on the stick, so the two can be compared |

Each step runs through [`results.py`](../../tools/mpc-one/results.py), so every log and report of a round lands in `~/mpfree-results/current/` and comes back as one zip ([how](../guides/survey-tools.md#one-folder-and-one-zip-per-test-round)).

## Saved settings (partition 4)

With `--app`, partition 4 (`mpfree-test-data`, an ext4 labelled `MPFREE-DATA`) holds the pad player's saved settings and files ([storage spec](../project/storage-spec.md)). It fills the rest of the stick, at least 1 GiB (plan decision 1, approved 2026-11-12).

- **Where it is.** It starts at 1 GiB on every build, so it can stay where it is from round to round. The builder stops if partitions 1 to 3 would reach past 1 GiB. `stick.img` keeps it small (64 MiB, empty), so the image stays about 1.1 GiB and mostly sparse.
- **What `write-test-stick.py` writes.** A partition table made for the stick it writes: partitions 1 to 3 as in `stick.img`, and partition 4 from 1 GiB to the last 1 MiB boundary before the backup table. Partition 4 itself is one of these:
  - **Left in place** when the stick already has MPFree's data partition at 1 GiB, which is every round after the first with this layout. It is not written at all, however big it has grown.
  - **Moved**, when the stick has the old layout (64 MiB, straight after partition 3). It is read into memory, checked and saved to `data-from-old-stick-<UTC>.img` in the build folder (a new file, mode 600, synced) before anything is written, then written at 1 GiB.
  - **Empty**: when the stick has none, with `--fresh-data`, or from a file with `--data-from FILE`.
  - **Stop, nothing written**, when the stick's partition 4 is not MPFree's, or is bigger than the room for it, or a `--data-from` file is not a whole `MPFREE-DATA` copy.
  - **Stop after a copy, nothing written on the stick**, when the stick has two different MPFree data partitions (different ext4 UUIDs): one at 1 GiB and another where its partition table says. That happens when a stick written with this layout is later rewritten by an older tool. The one in the table is what the MPC used last, so it is saved to the build folder for `--data-from`, and the message gives both partitions' last-written times. A partition moved by a cut-off run has the same UUID as the original, so it is not counted as a second one.
- **Growing the filesystem.** At boot, after partition 4 is mounted, `start-app.sh` runs `/usr/sbin/resize2fs` on it in the background, which grows a mounted ext4 to fill its partition (online, writing only partition 4). Output goes to `/run/mpfree-data-grow.txt`; a failure leaves the filesystem at its old size, logs one line to the kernel log, and the pad player saves as before. The builder says whether this release's root filesystem has `/usr/sbin/resize2fs`. **Not verified yet:** that 3.10.0 has it, and that its kernel grows the filesystem online. The device round checks both (`df /run/mpfree-data` after boot). The filesystem is made with 4 KiB blocks and room to grow online to 256 GiB.

**Write order, and what a cut-off write leaves** (the stick pulled out, the workstation stopped). The tool runs again safely after any of them, and the tests cut the write off at every step and run it again (`tests/test_write_test_stick.py`).

| Step | What is written | If it is cut off here |
| --- | --- | --- |
| 0 | Nothing on the stick. A moved partition 4 is saved to `data-from-old-stick-<UTC>.img` first | The stick is as it was |
| 1 | A new partition 4 at 1 GiB (moved, empty or `--data-from`). Its first 4 KiB are cleared first and written last, the superblock's first sector very last. A partition left in place skips this step | The stick keeps its old partition table, and the old partition 4 where it was (still whole if it ends before 1 GiB, as the old layout's does when partitions 1 to 3 fit below 1 GiB). The half-written one is never taken for MPFree's (its superblock is zero or its checksum fails). Running again moves it again |
| 2 | Partitions 1 to 3 from `stick.img`, from the end of the main table, so the gap before partition 1 holds the image's zeros | Partition 4 is whole at 1 GiB. Running again finds it in place |
| 3 | The backup partition table at the end of the stick, then the main one at the start | The same. The MPC may not boot from the stick until it is written again |

Then it reads every written region back and compares it with what it meant to write.

**Undo.** The stick's old contents are gone; nothing on the workstation or the MPC changes. Saved settings that were moved are in the build folder: `python3 tools/mpc-one/write-test-stick.py --data-from ~/mpfree-work/kexec-stick-<UTC>-*/data-from-old-stick-<UTC>.img` writes them back. Settings that were left in place stay on the stick until `--fresh-data`. To use the stick for something else, reformat it (for example with GNOME Disks).

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
- **Fixes for the next boot:** the PHY's ID in `kexec.dtb` ([which device tree](#which-device-tree)), and the on-screen log ([the screen](#the-screen)). `offline-kexec-details.py` showed the PHY's reset line in both board files and confirmed that neither kernel has a framebuffer console.
- **Reserved memory:** 3.10.0's kernel skipped U-Boot's `framebuffer` and `dma-unusable` regions as `invalid reg property`, after Akai's own fixup converted them, so DRM had no reserved framebuffer. That only affects the splash hand-over.

## Second boot (2026-11-10)

- **Storage:** the hashes of p1–p6 and the boot areas match before and after, and match the first baseline of 2026-11-09.
- **Screen:** works. The title, the status row (`up 98 s | network: eth0 down, sit0 down`) and the live log appeared, the right way up (photo from the owner).
- **Ethernet:** the PHY's ID fixed the first problem: `PHY [stmmac-0:01] driver [RTL8211E Gigabit Ethernet] (irq=54)`. The network service then opened `eth0` at 7.4 s, and the controller's DMA software reset timed out: `Failed to reset the dma`, `stmmac_hw_setup: DMA engine initialization failed`. That reset finishes only when the controller's clocks run, and with `clock_in_out = "input"` its 125 MHz clock and the receive clock come from the PHY. **Inferred, not verified:** the PHY is not giving those clocks after the jump. The upstream 6.6 and 6.18 code on this path (Rockchip MAC glue, DMA reset, RK3288 clocks, RTL8211E driver, PHY reset handling) is the same, so the difference is the hardware state, not a setting.
- **Note for later rounds:** the first boot's `MDIO device at address 1 is missing` means the MDIO reads finished and returned all ones, so the controller's register clock worked and only the PHY was silent.
- **Next:** the Ethernet details from 3.6.0 (step 9) and from the test build at several moments (before the network starts, after it fails, and after each of three retries of `ip link set eth0 up` at 20, 45 and 90 s), to compare the PHY reset line, the clock tree and the pins.

## Third boot (2026-11-10)

- **Storage:** the hashes of p1–p6 and the boot areas match before and after, and match the first baseline of 2026-11-09. 3.6.0 wrote 0 sectors to p1–p6 before the jump.
- **Ethernet and SSH work, after one retry.** The network service opened `eth0` at 7.78 s, and the DMA reset timed out again at 7.99 s, as on the second boot. The test build's retry (`ip link set eth0 up`) at 20.6 s worked: the PHY attached, the link came up at 23.1 s (`Link is Up - 1Gbps/Full`), DHCP gave an address at 28.0 s, and the workstation's SSH check logged in at 32.9 s. The MDIO bus made 150 transfers with 0 errors.
- **Measured, 3.6.0 against the test build:** the controller's clocks (`ext_gmac` → `mac_clk` → `sclk_mac_tx` on, `sclk_mac_rx` off in both, because RGMII takes its receive clock from a PHY pin), its pins, and the PHY reset line (output high) are the same in 3.6.0 and in the test build at 4.5 s, before the first start. They are also the same before the failed start and after the retry. Nothing the kernel can show explains the failure.
- **What differs between the failed start and the retry:** `systemctl kexec` stops 3.6.0's network service, and taking the interface down powers the PHY down and holds it in reset across the jump (inferred from the kernel code and the first boot's silent PHY). The test build's kernel releases it when the controller probes (about 0.2 s), and the PHY then runs on its own until the first start at 7.78 s. The failed start ends in `phy_detach`, which powers the PHY down and holds it in reset again (the reset line read low at 15 s). The retry releases it, waits the device tree's 30 ms and configures it, and the DMA reset has finished about 15 ms later.
- **Inferred, not verified:** the DMA reset needs the PHY's receive clock, and after 7.5 s on its own the PHY is not giving it, while a PHY fresh out of reset does. Two candidates, neither checked: the PHY put the idle link into Energy Efficient Ethernet's low-power state and stopped the receive clock, or the PHY's clock outputs do not settle after the quick reset pulses at probe.
- **Workaround:** the test build's retry. A system of ours started by kexec will meet the same failure, so it needs either the same retry or a PHY reset just before the first start.

## Keep it private

The `kexec-stick-*` folder and the stick hold Akai's root filesystem. They also hold a test SSH host key, your public key, and, in `kexec.dtb`, the unit's serial number and MAC. Never commit or share them. The builder's `report.txt` names the changes only, and is fine to share.
