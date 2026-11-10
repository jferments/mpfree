# Reading Akai's updater offline

**Status:** run by the owner on the real backup on 2026-11-10. Both hashes matched the boot map. [Findings](#findings-2026-11-10) below.

Before the reference unit takes any official update, and before the Tier 2 gate in the [recovery strategy](../engineering/RECOVERY_AND_OS_STRATEGY.md), we need to know what an Akai update writes: which partitions, in what order, and what it checks first. Part of the answer is on the MPC already: `/opt/ramfs-update.squashfs` sits in the rootfs (`p6`), and the [eMMC backup](emmc-backup.md) holds a verified copy of it.

## What it does

[`offline-akai-updater.sh`](../../tools/mpc-one/offline-akai-updater.sh) runs **on the workstation only**. It makes no SSH or network connection, so the MPC doesn't need to be on.

1. Picks the newest verified backup (a folder with `SHA256SUMS`) in `~/mpfree-backups`.
2. Copies the `p6` region of `mmcblk0.img` into a new work folder and checks it against the `p6` hash in the [boot map](MPC_ONE_3_6_0_BOOT_MAP_20261109.md). It stops if they differ. The backup itself is only read.
3. Reads that copy with `debugfs`, which is read-only unless given `-w` (never used). It copies out:
   - `/opt/ramfs-update.squashfs`, checked against its boot-map hash
   - `/boot/boot.scr.uimg`, U-Boot's boot script
   - every unit in `/usr/lib/systemd/system` whose name contains `update`, `upd`, `flash` or `fw`
4. Lists and unpacks the squashfs with `unsquashfs`.
5. Writes `report.txt`. Scripts and unit files are printed in full. Binaries are summarised by the strings that mention storage, partitions or update steps.
6. Deletes the `p6` copy and keeps the extracted files.

## Running it

From an up-to-date checkout:

```bash
bash tools/mpc-one/offline-akai-updater.sh
```

It needs `debugfs` (e2fsprogs), `unsquashfs` (squashfs-tools), `file` and `strings` (binutils), and about 550 MB free under `~/mpfree-work` while it runs. Exit status 0 means the report is complete. Exit status 1 means it stopped; the last line says why.

Options: `--backup DIR` picks a specific backup, and `MPFREE_WORK_DIR` changes the output location. The work folder must not be inside the backup, or inside the git clone unless git ignores it. The script checks both before creating anything.

## Findings (2026-11-10)

From the owner's run on the reference unit (3.6.0, `MOD-001+MOD-002`). The `p6` copy and `ramfs-update.squashfs` both matched their boot-map hashes. The report and extracted files stay private. Everything about behaviour below is **inferred from the binary's strings**. Nothing was executed or disassembled.

**There are no update scripts.** `ramfs-update.squashfs` is a small RAM-only Linux userspace: Akai's "az0x base distribution 5.0.8" (Yocto), with BusyBox, util-linux `mount`, glibc and OpenSSL `libcrypto`. Its `/usr/sbin/init` is a 58,808-byte compiled program, inMusic's own updater. The file dates (2018-03-09) are Yocto's fixed build timestamp, not a real date.

**What the updater's strings say it does** (inferred):

| Step | Evidence (strings) | What it means for us |
| --- | --- | --- |
| Mounts `/proc`, `/sys`, a tmpfs at `/mnt`, then an "upgrade device" | `failed to mount upgrade device`, `trying to mount %s as '%s'` | The update image comes from a mounted device or filesystem. Which one isn't known yet |
| Checks a signature before writing | `attempting to verify signature with key %s`, `v3 signature verified, checking compatibility`, OpenSSL `EVP_DigestVerify` with SHA-256 | Update images are signed. MPFree can't use this path to install anything, which is expected |
| Checks the image's partitions | `verifying image partitions`, `verifying image (%s)...` | The image carries a per-partition list |
| Writes partitions by name, optionally compressed | `write partition %s, compression=%s`, `writing partition %s (%zu bytes)`, `partition not found: %s`, `refusing to lookup partition %s` | It writes whole partitions from the image, looked up by name |
| Can rewrite the bootloader, and skips it when unchanged | `u-boot`, `uboot`, `uboot1`, `uboot2`, `bootloader unchanged!` | **An official update can write the brick surface (`p1`/`p3`)**, probably only when the image's bootloader differs |
| Can erase the data partition | `erase-data-partition`, `erase partition %s, device=%s`, `failed to erase data partition` | **An update can wipe `p8` (`/data`)** if the image asks for it. Anything MPFree keeps in `/data/mpfree` must also have a copy elsewhere |
| Finds the eMMC by its controller path | `%s/mmc2/mmc2:0001/block/mmcblk2`, `%s/mmcblk%lu` | The running 6.6 kernel names the eMMC `mmc0`/`mmcblk0`. Whether this path is old or still used is unknown |

**Other findings:**

- `/usr/bin/az01-update` does **not** exist in `p6`. An earlier note in the boot-map doc said it did. Neither boot-map run shows it, so that note was wrong and is corrected.
- What starts the RAM updater is unknown. None of the update-named units mentions it, and the rest of `p6` hasn't been searched yet. The MPC app itself is a plausible launcher (for example for an update from a USB drive), but that is a guess.
- `boot.scr.uimg` has no update branch. It loads `zImage` and the board DTB (plus an `az01b` overlay on that board) and boots with `fsck.repair=yes`. Update Mode stays U-Boot's own `fastboot 0`, as the [boot map](MPC_ONE_3_6_0_BOOT_MAP_20261109.md) found.
- The other `update`/`fw` units are stock systemd ones, plus Akai's `touch-fw-update.service` (Ilitek touch firmware) and `xmos-update.service`, which only runs when the device tree has no `sound` node. This unit uses I²S audio, so it probably never runs here.

**There are two update paths.** The desktop updater on a computer writes over USB in Update Mode, which is U-Boot fastboot. Standard U-Boot fastboot can flash any GPT partition by name, including `uboot-spl` and `uboot`, but whether Akai's build allows that is unverified. The RAM updater above writes from inside Linux and handles U-Boot itself. Which partitions a real 3.6.0 update contains still needs Akai's public update file. Its partition list may be readable without running anything.

**Still open:**

1. Find what launches the RAM updater: search the whole `p6` copy for references to `ramfs-update`.
2. Read the partition list of Akai's public 3.6.0 update file.

## Keep Akai's files private

The work folder `~/mpfree-work/akai-updater-<UTC time>-<random>/` holds Akai's proprietary software, and `report.txt` quotes their scripts in full. Treat both like the backup: never commit, upload or share them publicly. Only our written findings go in the repo, such as which partitions an update writes and what it checks.

## What it does not cover

- **Akai's downloadable 3.6.0 update.** It may differ from the copy on the device, and reading it is a separate step.
- **Running anything.** Nothing from the updater is executed, mounted or loop-attached.
