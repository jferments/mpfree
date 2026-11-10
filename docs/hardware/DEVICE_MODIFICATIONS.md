# Reference MPC One: record of persistent device modifications

**Purpose:** a reference unit that is described as "stock" but isn't makes every
recovery and compatibility claim unreliable. This file lists every *persistent*
change made to the owner's reference MPC One (firmware 3.6.0), with enough
detail to verify it and undo it. Volatile `/run`/`/tmp` experiments
(M5 to M10, GPU qualification) are not listed: they leave nothing behind after a reboot.

Add a row here **before** (or in the same PR as) any future persistent change.

## Current state: NOT stock

| ID | Date | Partition | Change | Status |
| --- | --- | --- | --- | --- |
| MOD-001 | 2026-11-09 (recorded) | `p6` rootfs (`/`) | Persistent SSH early-boot unit: `/usr/lib/systemd/system/mpfree-sshd.service` plus enable link `/usr/lib/systemd/system/multi-user.target.wants/mpfree-sshd.service` | **Active; owner decision: keep** |
| MOD-002 | 2026-11-09 (recorded) | `p8` data (`/data`) | Persistent SSH configuration and keys under `/data/mpfree/ssh/` | **Active; owner decision: keep** |

### MOD-001 / MOD-002: persistent SSH

**Why:** USB autorun SSH (see [DEVELOPER_ACCESS.md](DEVELOPER_ACCESS.md)) had to be
re-run after every reboot. Persistent SSH is now the primary way code is
deployed and tested on the MPC.

**Why the unit is in `/usr/lib` and not `/etc`:** the writable `/etc` overlay
(upperdir on `/data/system/etc/overlay`) becomes visible only after systemd has
already built part of its boot dependency graph. A unit installed under
`/etc/systemd/system` started after a manual `daemon-reload` but was **not**
activated at boot. Putting the unit and its `.wants` link in the base rootfs
makes it visible early.

**How it was installed:** `/` was temporarily remounted read-write, the two
rootfs paths above were created, and `/` was returned to read-only. The unit
then started correctly after a reboot and after the Ethernet cable was
unplugged and reconnected.

**Access:** key-only root SSH on TCP `2222` (secondary listener on `2223`) at the
reference lab address (`10.42.0.141`). The private key stays on the workstation.
Never commit key material or host keys.

**Risk this carried:** the rootfs is ext4 mounted **without a journal**, so a power loss during the read-write window could have corrupted it. Remounting the read-only, ~96%-full rootfs and writing
to it is a boot-critical write (Tier 3 in the
[recovery strategy](../engineering/RECOVERY_AND_OS_STRATEGY.md)). It succeeded,
and the unit has booted cleanly since. That success does not make it a pattern
to repeat. The next change to the rootfs or boot partitions waits for the Tier 3 gate.

**Security:** a root SSH daemon now starts on every boot. Keep it key-only and
on the direct lab Ethernet link. Remove it before the unit is used on another network
or handed to someone else.

**Verification:** `bash tools/mpc-one/collect-over-ssh.sh --boot-map` prints the
SHA-256 of both rootfs paths and the names (not contents) under
`/data/mpfree/ssh/`. Record the hashes in this file after the first run:

| Path | SHA-256 |
| --- | --- |
| `/usr/lib/systemd/system/mpfree-sshd.service` (375 bytes, mtime 2026-11-09 19:49) | `062881f29f27acca7dc1434538af841073f6ccf101bc4c6b9fd66714ec5f9eaa` |
| `multi-user.target.wants/mpfree-sshd.service` | symlink → `../mpfree-sshd.service` (same hash) |
| `p6` rootfs whole-partition hash with MOD-001 | `fa2ca12bfac82f5ca69238f8b85c1f1df18994486b385086ebdda26ae61b2350` |

Recorded from the owner's `--boot-map` run on 2026-11-09 22:04 UTC. The unit was `enabled` and `active`. `/data/mpfree/ssh/` held `authorized_keys`, `host_ed25519`, `host_ed25519.pub` and `sshd_config` (only the names were read).

**Effect of an official Akai firmware update:** an update rewrites `p6`, so it
will almost certainly **remove MOD-001**. SSH then falls back to USB autorun,
if autorun still works on the new firmware. MOD-002 lives on `/data` and will
probably survive, so delete it by hand. It is also possible that Akai's
updater checks `p6` before patching and refuses a modified rootfs. Until the
update scripts in `/opt/ramfs-update.squashfs` have been read offline, **do not
update the reference unit.**

**Undo, preferred:** reinstall the official 3.6.0 image through Akai's
documented Update Mode. This restores a clean `p6` and doubles as the Tier 2
recovery rehearsal. Then remove `/data/mpfree/ssh/`.

**Undo, minimal (also a rootfs write; avoid unless needed):** remount `/`
read-write, delete the two MOD-001 paths, then remount `/` read-only and
reboot. No script for this is provided on purpose.

## Consequences for backups

There is no pristine `p6` on this unit any more. A full eMMC backup taken now
captures the **modified** rootfs. A clean stock rootfs can only come from
Akai's official update image. Label backups accordingly
(`state=MOD-001+MOD-002`). The first full backup (2026-11-09, [eMMC backup](emmc-backup.md#first-run-2026-11-09))
carries that label in its manifest.
