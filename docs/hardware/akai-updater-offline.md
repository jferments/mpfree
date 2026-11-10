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

- `/usr/bin/az01-update` does **not** exist in `p6`. An earlier note in the boot-map doc said it did. Neither boot-map run shows it, so that note was wrong and is corrected.
- **What starts the RAM updater: `/usr/lib/libaz01-prog.so.1.3`**, an inMusic library. It is the only file in `p6` whose contents mention `ramfs-update` or `erase-data-partition` (owner's second run, 2026-11-10). See [How an update is launched](#how-an-update-is-launched).
- `boot.scr.uimg` has no update branch. It loads `zImage` and the board DTB (plus an `az01b` overlay on that board) and boots with `fsck.repair=yes`. Update Mode stays U-Boot's own `fastboot 0`, as the [boot map](MPC_ONE_3_6_0_BOOT_MAP_20261109.md) found.
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

- **Akai uses kexec on this board for every USB and online update.** It boots its own kernel from the running system with a RAM disk, which is the route we planned for testing our own kernel in RAM ([recovery strategy](../engineering/RECOVERY_AND_OS_STRATEGY.md), option 1). It doesn't prove our kernel will come up cleanly, but it shows this hardware and kernel handle kexec in normal use.
- **There are three ways an update reaches the eMMC.** The desktop updater writes over USB in Update Mode (U-Boot fastboot). Standard U-Boot fastboot can flash any GPT partition by name, including `uboot-spl` and `uboot`, but whether Akai's build allows that is unverified. USB-drive and online updates run the RAM updater above, which can write U-Boot itself and erase `/data`.
- **Updates and factory reset need mains power.** The library refuses both without it, which lowers the risk of a half-written bootloader.

**Still open:** read the partition list of Akai's public 3.6.0 update file with `offline-update-image.py` (below). It shows whether a real update carries a bootloader payload at all.

## Reading Akai's public update file

[`offline-update-image.py`](../../tools/mpc-one/offline-update-image.py) lists what a downloaded update image contains, also on the workstation only. inMusic's update images are flattened device tree (FDT/FIT) containers: a tree of nodes whose properties hold each partition's payload, hashes and signatures. The tool prints that tree with every property's name and size, short values in full, and a SHA-256 and format guess (gzip, xz, zstd, ext4, Rockchip loader, U-Boot image and so on) for each large payload. It doesn't decompress, mount or run anything, and it never writes the image. A `.zip` download is unpacked into a temporary folder next to it, which is deleted afterwards.

```bash
python3 tools/mpc-one/offline-update-image.py ~/mpfree-work/akai-update-files
```

It reads every `.img` and `.zip` directly inside that folder. Keep downloaded images outside the git clone, like the backup.

## Keep Akai's files private

The work folder `~/mpfree-work/akai-updater-<UTC time>-<random>/` holds Akai's proprietary software, and `report.txt` quotes their scripts in full. Treat both like the backup: never commit, upload or share them publicly. Only our written findings go in the repo, such as which partitions an update writes and what it checks.

## What it does not cover

- **Akai's downloadable 3.6.0 update.** It may differ from the copy on the device, and reading it is a separate step.
- **Running anything.** Nothing from the updater is executed, mounted or loop-attached.
