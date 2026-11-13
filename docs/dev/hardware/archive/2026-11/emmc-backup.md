# Full eMMC backup over SSH (`--backup`)

**Status:** first owner run **verified** on 2026-11-09 (see [First run](#first-run-2026-11-09)). Tested against a fake device in CI.

A verified, off-device copy of the whole eMMC is the first condition of the Tier 1 recovery gate in the [recovery strategy](../../../research/recovery-and-os-strategy.md), and every later recovery path ends with "write the backup back." The reference unit is not stock ([device modifications](device-modifications.md)), so Akai's installer cannot recreate its current state. Only this backup can.

## What it does on the MPC

**Read-only.** After the usual interface, route, MAC and host-key checks, `collect-over-ssh.sh --backup` sends exactly five fixed commands over SSH:

| Command | What it reads |
| --- | --- |
| `sh -s hash` with [`read-only-backup-preflight.sh`](../../../../../tools/mpc-one/read-only-backup-preflight.sh) | Partition geometry, eMMC model, block-layer write counters, `boot_id`, and SHA-256 of sectors 0–63 (MBR and primary GPT), `p1`–`p6`, `boot0` and `boot1` via `dd if=… \| sha256sum`. Also EXT_CSD, the eMMC boot configuration, but only if debugfs is already mounted |
| `dd if=/dev/mmcblk0 bs=1048576` | The whole user area, 7,820,083,200 bytes, sent to SSH stdout |
| `dd if=/dev/mmcblk0boot0 bs=1048576` | Hardware boot area 0 (4 MiB, all zeros on this unit) |
| `dd if=/dev/mmcblk0boot1 bs=1048576` | Hardware boot area 1 (4 MiB, all zeros on this unit) |
| `sh -s stat` | `boot_id` and write counters again |

Nothing is mounted, written, synced or restarted. No `dd` has an output file on the MPC. CI checks the fixed command list, scans both scripts for write operations, and runs the whole flow against a fake MPC under BusyBox, including a fake `ssh` that rejects any other command.

The first full copy took 193 s at 40.5 MB/s, so expect about 4 minutes including the preflight. The MPC's own app keeps running, but keep it idle (no playback or recording) while the copy runs.

## Running it

From an up-to-date checkout, on the workstation:

```bash
bash tools/mpc-one/collect-over-ssh.sh --backup
```

Backups go to `~/mpfree-backups/`, **outside the git clone**, so they can't be committed by accident or deleted by `git clean -fdx`. You need about 7.6 GiB free there, and the tool checks before copying. Set `MPC_BACKUP_DIR=/some/other/disk` to write somewhere else. The wrapper refuses a folder inside the clone unless git ignores it.

| Exit status | Meaning |
| --- | --- |
| 0 | `BACKUP VERIFIED`. Every check passed |
| 1 | `BACKUP FAILED` or not started. Partial images are deleted, `MANIFEST-FAILED.txt` says why. Nothing was written on the MPC |
| 3 | The backup is verified against the MPC and kept, but boot-critical storage **differs from the 2026-11-09 boot map**. Stop and report before doing anything else on the MPC |
| 130 | Interrupted (Ctrl-C or timeout). Partial images are deleted |

## Checks before an image is kept

1. **Layout:** the disk size, every partition's start, length and name, the boot areas, and the eMMC model must match the [reference baseline](../../../../../tools/mpc-one/baselines/mpc-one-3.6.0-reference.txt) exactly, or nothing is copied.
2. **Same bytes as the MPC sees:** the copied stream's MBR/GPT, `p1`–`p6`, `boot0` and `boot1` regions must hash the same as the MPC's own hashes from step 1.
3. **No changes during the copy:** write and discard counters for `p1`–`p6`, `boot0` and `boot1` must not move between the first counter read and the end of the copy, and the MPC must not reboot.
4. **Workstation disk:** each image is re-read from disk, with that file's page cache dropped, and must match the hash computed while receiving.
5. **Baseline:** `p1`, `p3`–`p6`, `boot0` and `boot1` are compared with the [boot map](boot-map.md) hashes (exit status 3 on a difference).

`p7` (content) and `p8` (data) are mounted read-write. Their write counters are reported in the manifest. If they changed during the copy, those partitions are captured like a power-loss snapshot. SSH's per-packet integrity check covers their bytes in transit.

## What you get

A new folder `~/mpfree-backups/mpc-one-emmc-<UTC time>-<random>/`, files read-only (mode 0400):

- `mmcblk0.img`: the whole user area, including both GPT copies and `p1`–`p8`
- `mmcblk0boot0.img`, `mmcblk0boot1.img`
- `SHA256SUMS`: check with `sha256sum -c SHA256SUMS`
- `MANIFEST.txt`: kernel, eMMC model, EXT_CSD, per-partition offsets and hashes, `p7`/`p8` write notes, the tool's git commit

Make a second copy on a different disk and check it there:

```bash
SRC="$(ls -d ~/mpfree-backups/mpc-one-emmc-*/SHA256SUMS | tail -n 1)" && \
SRC="${SRC%/SHA256SUMS}" && \
DEST=/path/to/other/disk && \
cp -a "$SRC" "$DEST/" && \
cd "$DEST/$(basename "$SRC")" && \
sha256sum -c SHA256SUMS
```

`SRC` is the newest folder that has a `SHA256SUMS`, which only verified backups get.

## First run (2026-11-09)

Owner-run at 23:36 UTC on the reference MPC One (firmware 3.6.0, state `MOD-001+MOD-002`). Log SHA-256 `6b69ad44dd357fd4d1109977be2e85d727396c7e9d161939f9c760e41e58f809` (kept privately on the workstation, not committed).

- **Verified.** All checks passed, exit status 0.
- `p1`, `p3`–`p6`, `boot0` and `boot1` match the [boot map](boot-map.md) hashes exactly. The MPC's own hashes of the MBR/GPT, `p1`–`p6` and both boot areas match the copied data.
- Zero writes or discards on `p1`–`p6` and both boot areas during the copy, and no reboot.
- `p7`: no writes during the copy. `p8`: 96 sectors (48 KiB) written during the copy, most likely the factory app saving settings or logs (not checked), so `p8` is captured like a power-loss snapshot.
- 7,457 MiB in 193 s (40.5 MB/s). The 4 MiB boot areas took under a second each.
- EXT_CSD was readable and is saved in the private manifest.
- A second copy on a separate disk passed `sha256sum -c`.

Whole-image, `p2`, GPT, `p7` and `p8` hashes are left out here because they cover per-device and user data.

## Keep it private

The image contains Akai's proprietary content, your projects and settings, the SSH host key and authorized keys under `/data/mpfree/ssh/`, and the MPC's serial number and MAC address (in the U-Boot environment). **It is private: never commit, upload or share it.** The default folder is outside the git clone, and `research/` and `*.img` are git-ignored as a second guard.

## What it does not cover

- **Restoring.** Writing an image back is a Tier 3 operation (MaskROM and `rkdeveloptool`). It isn't provided, and needs its own gate.
- **RPMB.** The eMMC's replay-protected area, if any, isn't readable without its key and isn't captured. Nothing seen so far suggests the stock boot uses it (unverified).
- **A consistent `/data` at file level.** If `p8` changed during the copy, a file-level `/data` backup is the better restore source. That belongs to the Tier 2 gate.
