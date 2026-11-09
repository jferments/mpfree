# Reading Akai's updater offline

**Status:** tool ready and tested against a fake backup in CI. Not yet run on the real backup.

Before the reference unit takes any official update, and before the Tier 2 gate in the [recovery strategy](../engineering/RECOVERY_AND_OS_STRATEGY.md), we need to know what an Akai update writes: which partitions, in what order, and what it checks first. The answer is on the MPC already. `/usr/bin/az01-update` and `/opt/ramfs-update.squashfs` sit in the rootfs (`p6`), and the [eMMC backup](emmc-backup.md) holds a verified copy of it.

## What it does

[`offline-akai-updater.sh`](../../tools/mpc-one/offline-akai-updater.sh) runs **on the workstation only**. It makes no SSH or network connection, so the MPC doesn't need to be on.

1. Picks the newest verified backup (a folder with `SHA256SUMS`) in `~/mpfree-backups`.
2. Copies the `p6` region of `mmcblk0.img` into a new work folder and checks it against the `p6` hash in the [boot map](MPC_ONE_3_6_0_BOOT_MAP_20261109.md). It stops if they differ. The backup itself is only read.
3. Reads that copy with `debugfs`, which is read-only unless given `-w` (never used). It copies out:
   - `/opt/ramfs-update.squashfs`, checked against its boot-map hash
   - `/boot/boot.scr.uimg`, U-Boot's boot script
   - `/usr/bin/az01-update`
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

## Keep Akai's files private

The work folder `~/mpfree-work/akai-updater-<UTC time>-<random>/` holds Akai's proprietary software, and `report.txt` quotes their scripts in full. Treat both like the backup: never commit, upload or share them publicly. Only our written findings go in the repo, such as which partitions an update writes and what it checks.

## What it does not cover

- **Akai's downloadable 3.6.0 update.** It may differ from the copy on the device, and reading it is a separate step.
- **Running anything.** Nothing from the updater is executed, mounted or loop-attached.
