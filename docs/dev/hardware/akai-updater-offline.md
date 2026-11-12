# Reading Akai's updater offline

**Status:** run by the owner on the real backup on 2026-11-10. Both hashes matched the boot map. [Findings](#findings-2026-11-10) below.

Before the reference unit takes any official update, and before the Tier 2 gate in the [recovery strategy](../research/recovery-and-os-strategy.md), we need to know what an Akai update writes: which partitions, in what order, and what it checks first. Part of the answer is on the MPC already: `/opt/ramfs-update.squashfs` sits in the rootfs (`p6`), and the [eMMC backup](emmc-backup.md) holds a verified copy of it.

## What it does

[`offline-akai-updater.sh`](../../../tools/mpc-one/offline-akai-updater.sh) runs **on the workstation only**. It makes no SSH or network connection, so the MPC doesn't need to be on.

1. Picks the newest verified backup (a folder with `SHA256SUMS`) in `~/mpfree-backups`.
2. Copies the `p6` region of `mmcblk0.img` into a new work folder and checks it against the `p6` hash in the [boot map](boot-map.md). It stops if they differ. The backup itself is only read.
3. Reads that copy with `debugfs`, which is read-only unless given `-w` (never used). It copies out:
   - `/opt/ramfs-update.squashfs`, checked against its boot-map hash
   - `/boot/boot.scr.uimg`, U-Boot's boot script
   - every unit in `/usr/lib/systemd/system` whose name contains `update`, `upd`, `flash` or `fw`
4. Lists and unpacks the squashfs with `unsquashfs`.
5. Searches the whole `p6` copy for the text `ramfs-update` and `erase-data-partition`, maps each match to the file that holds it (`debugfs icheck` and `ncheck`), copies those files and prints the lines or strings around each match. Matches in free space (deleted data) or filesystem metadata (such as a short symlink target) are counted separately.
6. Writes `report.txt`. Scripts and unit files are printed in full. Binaries are summarised by the strings that mention storage, partitions or update steps.
7. Deletes the `p6` copy and keeps the extracted files.

## Running it

From an up-to-date checkout:

```bash
bash tools/mpc-one/offline-akai-updater.sh
```

It needs `debugfs` (e2fsprogs), `unsquashfs` (squashfs-tools), `file`, `strings` (binutils) and `python3`, and about 550 MB free under `~/mpfree-work` while it runs. Exit status 0 means the report is complete. Exit status 1 means it stopped; the last line says why.

Options: `--backup DIR` picks a specific backup, and `MPFREE_WORK_DIR` changes the output location. The work folder must not be inside the backup, or inside the git clone unless git ignores it. The script checks both before creating anything.

## Findings (2026-11-10)

From the owner's run on the reference unit (3.6.0, `MOD-001+MOD-002`). The `p6` copy and `ramfs-update.squashfs` both matched their boot-map hashes. The report and extracted files stay private. Everything about behaviour below is **inferred from the binary's strings**. Nothing was executed or disassembled.

**There are no update scripts.** `ramfs-update.squashfs` is a small RAM-only Linux userspace: Akai's "az0x base distribution 5.0.8" (Yocto), with BusyBox, util-linux `mount`, glibc and OpenSSL `libcrypto`. Its `/usr/sbin/init` is a 58,808-byte compiled program, inMusic's own updater. The file dates (2018-03-09) are Yocto's fixed build timestamp, not a real date.

**What the updater's strings say it does** (inferred):

| Step | Evidence (strings) | What it means for us |
| --- | --- | --- |
| Mounts `/proc`, `/sys`, a tmpfs at `/mnt`, then an "upgrade device" | `failed to mount upgrade device`, `trying to mount %s as '%s'` | The update image comes from a mounted device, passed in by the launcher (below) |
| Checks a signature before writing | `attempting to verify signature with key %s`, `v3 signature verified, checking compatibility`, OpenSSL `EVP_DigestVerify` with SHA-256 | Update images are signed. MPFree can't use this path to install anything, which is expected |
| Checks the image's partitions | `verifying image partitions`, `verifying image (%s)...` | The image carries a per-partition list |
| Writes partitions by name, optionally compressed | `write partition %s, compression=%s`, `writing partition %s (%zu bytes)`, `partition not found: %s`, `refusing to lookup partition %s` | It writes whole partitions from the image, looked up by name |
| Can rewrite the bootloader, and skips it when unchanged | `u-boot`, `uboot`, `uboot1`, `uboot2`, `bootloader unchanged!` | **An official update can write the brick surface (`p1`/`p3`)**, probably only when the image's bootloader differs |
| Can erase the data partition | `erase-data-partition`, `erase partition %s, device=%s`, `failed to erase data partition` | **An update can wipe `p8` (`/data`)** if the image asks for it. Anything MPFree keeps in `/data/mpfree` must also have a copy elsewhere |
| Finds the eMMC by its controller path | `%s/mmc2/mmc2:0001/block/mmcblk2`, `%s/mmcblk%lu` | The running 6.6 kernel names the eMMC `mmc0`/`mmcblk0`. Whether this path is old or still used is unknown |

**Other findings:**

- **`az01-update` exists, at `/usr/sbin/az01-update`** (found by the rootfs survey, 2026-11-10, in both 3.6.0 and 3.10.0). The boot map listed the name without a folder. An earlier version of this page looked only in `/usr/bin` and wrongly said the program doesn't exist.
- **What starts the RAM updater: `/usr/lib/libaz01-prog.so.1.3`**, an inMusic library. It is the only file in `p6` whose contents mention `ramfs-update` or `erase-data-partition` (owner's second run, 2026-11-10). See [How an update is launched](#how-an-update-is-launched).
- `boot.scr.uimg` has no update branch. It loads `zImage` and the board DTB (plus an `az01b` overlay on that board) and boots with `fsck.repair=yes`. Update Mode stays U-Boot's own `fastboot 0`, as the [boot map](boot-map.md) found.
- The other `update`/`fw` units are stock systemd ones, plus Akai's `touch-fw-update.service` (Ilitek touch firmware) and `xmos-update.service`, which only runs when the device tree has no `sound` node. This unit uses I²S audio, so it probably never runs here.

### How an update is launched

From the strings of `libaz01-prog.so.1.3`, in the order they appear (inferred, not run):

- **Update Mode** (Menu > Preferences > Info > Update > "Switch to Update Mode") is most likely `systemctl reboot --reboot-argument=loader`. U-Boot then starts `fastboot 0`. The `loader` → recovery-flag step happens in the bootloader and wasn't seen directly.
- **USB-drive and online updates** use **kexec**. The library refuses unless the MPC is on mains power (`refusing to update: not on AC power`). It verifies the image and checks it against the board (`image is not compatible with this device`, device-tree `compatible` and `inmusic,az0x-pcb-rev` nodes). Then it loads Akai's own kernel with the updater as a RAM disk and jumps to it:
  - `/usr/sbin/kexec --load /boot/zImage --ramdisk /opt/ramfs-update.squashfs --kexec-syscall-auto`
  - command line `ramdisk_size=32768 root=/dev/ram0 ro -- upgrade-device=… upgrade-file=…`
  - log lines `preparing upgrade environment...`, `booting upgrade environment...`
- **Factory reset** goes the same way with `erase-data-partition`, and also refuses without mains power (`refusing to erase user data: not on AC power`).
- Other reboot arguments are present: `update`, `factory-reset` and `update:%s:%s`. What U-Boot does with them is unknown.

The flag list and argument order are reconstructed from neighbouring strings, so the exact command line is an inference.

**What this means for MPFree:**

- **Akai uses kexec on this board for every USB and online update.** It boots its own kernel from the running system with a RAM disk, which is the route we planned for testing our own kernel in RAM ([recovery strategy](../research/recovery-and-os-strategy.md), option 1). It doesn't prove our kernel will come up cleanly, but it shows this hardware and kernel handle kexec in normal use.
- **There are three ways an update reaches the eMMC.** The desktop updater writes over USB in Update Mode (U-Boot fastboot). Standard U-Boot fastboot can flash any GPT partition by name, including `uboot-spl` and `uboot`, but whether Akai's build allows that is unverified. USB-drive and online updates run the RAM updater above, which can write U-Boot itself and erase `/data`.
- **Updates and factory reset need mains power.** The library refuses both without it, which lowers the risk of a half-written bootloader.

## Reading Akai's public update file

These workstation-only tools read a downloaded update image. None of them contacts the MPC, and none writes the image.

| Tool | What it does | Writes |
| --- | --- | --- |
| [`offline-update-image.py`](../../../tools/mpc-one/offline-update-image.py) | Lists every field of the container, with a SHA-256 and format guess for each payload | Nothing |
| [`offline-update-extract.py`](../../../tools/mpc-one/offline-update-extract.py) | Checks each payload against the image's own hashes, then decompresses it | A new folder `~/mpfree-work/akai-unpacked-<UTC>-<random>/` |
| [`offline-rootfs-survey.sh`](../../../tools/mpc-one/offline-rootfs-survey.sh) | Reads a rootfs image with `debugfs` (read-only) and reports what it mounts, which units start at boot, its flashing programs, Akai's own programs and scripts, firmware hashes, and which files mention the eMMC or flashing. Section 8 covers what a kexec boot needs: the drivers built into the kernel, the kernel's embedded config, and the board's device trees | A new folder `~/mpfree-work/rootfs-survey-<UTC>-<random>/`, and a copy of `/boot` in `~/mpfree-work/boot-files/<kernel release>/` (never overwritten) |
| [`offline-kernel-config.py`](../../../tools/mpc-one/offline-kernel-config.py) | Prints the build config embedded in a zImage (`CONFIG_IKCONFIG`), all of it or only the symbols that matter for a kexec boot | Nothing |
| [`offline-dtb.py`](../../../tools/mpc-one/offline-dtb.py) | Lists, summarises, compares and shows device trees. A comparison leaves values out unless asked, and `show` never prints the MAC address or serial number, so both are safe on the tree read from the MPC | Nothing |

```bash
python3 tools/mpc-one/offline-update-image.py ~/mpfree-work/akai-update-files
python3 tools/mpc-one/offline-update-extract.py ~/mpfree-work/akai-update-files/MPC-3.10.0-Gen1-update.img
bash tools/mpc-one/offline-rootfs-survey.sh                 # the rootfs just extracted
bash tools/mpc-one/offline-rootfs-survey.sh --from-backup   # the 3.6.0 rootfs (p6), for comparison
```

The extractor writes a payload only if one of the image's hashes matches it, either over the stored bytes or over the uncompressed data. A `.zip` download is unpacked into a temporary folder next to it, which is deleted afterwards. Keep downloaded images outside the git clone, like the backup.

### Two container formats

Older MPC images (and the Denon/inMusic devices documented publicly) are FDT/FIT containers: a device tree whose nodes hold each payload, its compression and its hashes.

The **3.10.0** image (`MPC-3.10.0-Gen1-update.img`, 167,084,848 bytes, sha256 `d09b1e4c…406a2a20`) uses a different format that starts with `AZ01`. The layout below is read from that file's first 340 bytes (owner's dump, 2026-11-10). Our test encoder rebuilds those bytes exactly. Everything is little-endian, and each string is a 32-bit length, the text, a NUL, then zero padding to a multiple of 4.

| Offset | Field | 3.10.0 value |
| --- | --- | --- |
| 0 | magic | `AZ01` |
| 4 | format version | 1 |
| 8 | header size | 264 |
| 12 | build | `SNAPSHOT-20261014152523` |
| 40 | compatible boards (count, strings) | 8: `inmusic,acv5`, `acv8`, `acva`, `acva2`, `acvb`, `acvm`, `acvr`, `acv5s` |
| 204 | device IDs (count, 32-bit values) | 8: `09e8:403a`, `403b`, `4046`, `4047`, `404b`, `4052`, `4056`, `4057` |
| 240 | description | `MPC upgrade image` |
| 264 | `PART` record: tag, record header size (76), 64-bit data size, name, compression, hash count, then per hash (algorithm, size, value) | `rootfs`, `xz`, one `sha1` |
| 340 | the partition data | an xz stream of 167,084,492 bytes |
| 167,084,832 | end record: tag `EOF\0`, record size 16, 8 zero bytes | end of file |

The owner's run of the lister and extractor (2026-11-10) confirmed the rest. The SHA-1 covers the stored (compressed) bytes and matches. The rootfs unpacks to 451,468,288 bytes (431 MiB, ext4), which fits the 500 MiB system partition.

What this means:

- **The 3.10.0 update carries only the rootfs.** There is no bootloader, splash or partition-table record, so this image can't rewrite U-Boot. The kernel and device trees live in the rootfs's `/boot`, as on 3.6.0.
- **Our unit is a target.** Its device tree says `inmusic,acva`, which is in the list. Its internal controller is USB `09e8:0046`. The listed `09e8:4046` looks like the same ID with `0x4000` added, which is an inference.
- **The file holds no signature.** Every byte belongs to the header, the one `PART` record or the `EOF` record. Where the "v3 signature" the RAM updater checks comes from is open.

### The 3.10.0 rootfs compared with 3.6.0

From `offline-rootfs-survey.sh` on the unpacked 3.10.0 rootfs and on the reference unit's `p6` (owner's run, 2026-11-10). These are file listings and unit files; nothing was run.

- **New kernel and base system.** 3.10.0 has kernel `6.18.44-az01-2026-08-10-rt6` (3.6.0: `6.6.95-az01-2025-06-27-rt55`) and Akai's base distribution 5.0.18 (3.6.0: 5.0.8). `/boot` has the same device-tree files, each a little larger.
- **The same mounts and update units, with these exceptions:**
  - New in 3.10.0: `/usr/bin/az01-bootloader-sig`, `/usr/sbin/az01config` and the boot service `az0x-loadpin-trustpoints.service`. What they do is in the next section.
  - 3.10.0 enables `az0x-info.service` and `az0x-webserver.service`, and mounts the content partition through a new `storage.mount`.
  - The reference unit's 3.6.0 rootfs enables a serial-console login on `ttyS2`, and 3.10.0 doesn't. Whether that is stock 3.6.0 is unknown.
- **Peripheral firmware updaters, in both versions.** `touch-fw-update.service` runs `touch-fw-upd` at every boot on boards with the ILI2117 touch controller at `i2c@ff160000` address `0x26`, which is this unit's. `acvs.service` (the MPC app) also mentions it. The app launcher `/usr/bin/az01-launch-MPC` and `/usr/share/Akai/SME0/Firmware/update.sh` mention `dfu-util`, so starting the app may flash a microcontroller over USB DFU. These write outside the eMMC, so a power-cycle wouldn't undo them. Any RAM-only test of another version must keep them from running.

### What Akai's 3.10.0 programs do

From the second survey run (owner's run, 2026-11-10), which prints Akai's scripts in full and the relevant strings of their programs. Nothing was run, so statements about programs are read from their strings and are inferences.

- **Touchscreen firmware: no change between versions.** `/lib/firmware/ilitek/ili2117.bin` is byte-identical in 3.6.0 and 3.10.0 (SHA-256 `0afa6ea3…`). `touch-fw-upd` reads the chip's version from `/sys/bus/i2c/devices/4-0026/fw_version`, compares it with the file and flashes through `update_fw` only when they differ. So 3.10.0's updater should find nothing to do on a unit that has run 3.6.0.
- **Controller (pad board) firmware is flashed only through the MPC app.** `az01-launch-MPC` runs `SME0/Firmware/update.sh` only if `dfu-util -l` already lists the controller in DFU mode (`0x08000000`). Putting it into DFU mode takes a SysEx message that `update.sh` sends with `amidi`, and that the app presumably sends when its version check fails (inference). `update.sh` can also flash `.rbin` files through `/usr/sbin/midifirmup`.
- **XMOS:** `xmos-update.service` is skipped on boards whose device tree has a `/sound` node (I2S audio), which includes the MPC One.
- **`az01config` writes "config storage" straight to eMMC partition 11, 4 or 3** (`/dev/disk/by-path/platform-ff0f0000.dwmmc-partN`). On the MPC One, `p3` is U-Boot and `p4` is the splash. Which partition it picks on this board is unknown, and no unit calls it. **This is one more reason not to install 3.10.0 on the reference unit.**
- **`az01-bootloader-sig` only reads**: it compares a "primary" and a "fallback" bootloader in partitions 1 and 2 and prints the signing-key hash. That layout isn't the MPC One's, where `p2` is the U-Boot environment. No unit calls it.
- **`az0x-webserver` (enabled in 3.10.0) installs updates sent over the network.** It accepts an upload to `/data/az01-update.img` and runs `az01-update --file` on it, after a PIN pairing step.
- **LoadPin:** `az0x-loadpin-trustpoints` loads `/usr/lib/az0x/trusted-verity-digests` into `/sys/kernel/security/loadpin/dm-verity` early in boot. LoadPin limits where the kernel loads modules and firmware from. It doesn't control `kexec_load`, which is what a test boot from 3.6.0 would use (inference from the upstream kernel).
- **SSH:** 3.10.0 ships `sshd` but doesn't start it. `az01-ssh-login` is a wrapper for certificate logins restricted by product code and serial number.

**For a RAM-only test boot of 3.10.0** this means: hide the eMMC controller from the 3.10.0 kernel in its device tree, so no program can reach any eMMC partition; disable kexec inside it (`kernel.kexec_load_disabled=1`), because Akai's updater and anything else would need kexec to reach a kernel that sees the eMMC; and keep the app, the web server and all firmware updaters from running. The full design, and the tools that build and write the USB stick for it, are in [Test-booting another firmware version by kexec](kexec-test-boot.md).

## Keep Akai's files private

The work folders under `~/mpfree-work/` (`akai-updater-…`, `akai-unpacked-…`, `rootfs-survey-…` and `boot-files/`) hold or quote Akai's proprietary software, and the reports quote their scripts in full. Treat both like the backup: never commit, upload or share them publicly. Only our written findings go in the repo, such as which partitions an update writes and what it checks.

## What it does not cover

- **Akai's downloadable 3.6.0 update.** Only 3.10.0 is offered for download, so the 3.6.0 rootfs is read from the backup instead.
- **Running anything.** Nothing from the updater or an update image is executed, mounted or loop-attached.
