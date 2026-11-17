# The SD card and the MPC's internal drive in MPFree (plan)

**As of 2026-11-17** · **Status:** plan, nothing built. · **Asked for by** the owner, 2026-11-17 00:10Z: "eventually I would like for MPFree to be able to utilize the SD card to store samples, etc. and I would like to be able to have read-only access to the eMMC as needed. So these should be something that our kernel has the capability to turn on/off. We need to come up with a plan to safely partition / use the SD card and share it with Akai factory OS."

## Goal

- **The SD card:** MPFree reads samples, kits and projects from the card in the MPC's SD slot, and saves its own files there. The same card keeps working in Akai's system and in a computer.
- **The internal drive (the eMMC):** MPFree can read the musician's files on it, **read-only**, when asked: Akai's Internal drive (partition 8) and Akai's factory content (partition 7). Nothing on the eMMC is ever written.
- **Off unless asked for:** both stay switched off at every start, as today. MPFree switches one on when the musician opens it, and off again afterwards and at shutdown.

## Where we are today

- **Every MPFree start switches both off before anything touches them.** On the soft-reboot route the stick unbinds the eMMC controller (`ff0f0000`) and the SD controller (`ff0c0000`) first and powers the MPC off if it can't ([how](soft-reboot-boot.md)). On the chain the stick's device tree disables both controllers, so the second kernel never sees them ([step 7](stick-autoboot.md#the-chain-to-the-sticks-own-device-tree-step-7-opt-in)).
- **One read-only window exists already: the first-start backup** on a creator stick (`tools/mpc-one/stick/setup-backup.sh`, [how](soft-reboot-boot.md)). It binds the eMMC controller alone, marks every eMMC disk read-only as it appears (a udev rule in `/run`, the kernel's `force_ro`), checks the partition table and the block layer's write and discard counters, copies the eMMC to the stick, checks the counters did not move, and unbinds the controller again. The SD controller stays off. It runs on Akai's kernel (the soft-reboot route), where the controllers are in the device tree; on a `--chain` stick's second kernel they are not, so the backup can't run there.
- **MPFree already reads ext4 without the kernel:** the USB creator's ext4 reader (`rust/mpfree-diskimage`) copies Akai's root filesystem out of the update file.
- **The plan for our own kernel said "no MMC driver at all".** That is corrected below: our kernel has the drivers, and nothing starts them unless MPFree asks.

What we know about Akai's side, from earlier rounds:

- Akai's disk service (`edisksd`) mounts **every partition it can read** on a removable disk, **read-write**, at `/media/<label or UUID>` (`nosuid,nodev,noexec,nosymfollow`); it did that to all of a stick's partitions, ext4 included ([record](stick-autoboot.md)).
- Akai's `no-reserved-blocks@.service` runs `tune2fs` on **every ext4 partition** as it appears, which writes that filesystem's superblock. It started for the SD card on 2026-11-15 ([record](stick-autoboot.md#second-power-on-2026-11-15-zip-092220z-mpfree-started)).
- The owner's card is a 64 GB card with one exFAT partition.
- Not known yet: whether Akai's system mounts an SD card the same way, which filesystems its kernel reads and writes (FAT32, exFAT, NTFS, ext4), what it does with a partition it cannot read (ignore it or offer to format it), whether the SD slot reports card insertion, and what Akai keeps on a card. Step S0 finds out.

## How the SD card is shared with Akai's system

This is the owner's choice (asked on 2026-11-17). Three ways:

| | **A. A folder on the card as it is** (recommended) | B. A partition of MPFree's own | C. A second card just for MPFree |
| --- | --- | --- | --- |
| The card | Unchanged: its one FAT32 or exFAT partition, as Akai or a computer made it | Akai's partition made smaller, plus an ext4 partition for MPFree | A card MPFree formats (ext4) |
| What MPFree writes | Only inside one top folder, `MPFree/` | Only its own partition | The whole card |
| Akai's system | Sees MPFree's folder; its samples are ordinary WAV files Akai can load | Mounts MPFree's partition too, read-write, and runs `tune2fs` on it | Mounts it if it is in while Akai runs |
| A computer | Reads everything | Can't read the ext4 partition (Windows, macOS) | Can't read it (Windows, macOS) |
| A power cut while saving | FAT and exFAT keep no journal, so a cut can damage the filesystem, the same risk Akai's own saves carry. MPFree lowers it: whole files written then renamed, synced, the card checked before writing | ext4's journal protects it | ext4's journal protects it |
| Getting there | Nothing to do | Repartitioning a card that has data on it means copying everything off and back (there is no safe exFAT shrink); never done without a typed confirmation | Formatting a card; never done without a typed confirmation |

A keeps one card that works everywhere, with nothing to repartition, and is the recommendation. B or C can be added later for musicians who want a journal.

## How MPFree switches them on and off

| | On Akai's kernel (soft-reboot route; the chain until our kernel) | On our own kernel (chain) |
| --- | --- | --- |
| At every start | Both controllers unbound first, as today | The MMC drivers are modules that nothing loads at start; both controllers are held (`driver_override`) so that loading the driver binds neither. Nothing probes the eMMC or the card at all |
| SD card on | Bind the SD controller alone (`drivers_probe`), as the backup does for the eMMC | Release the SD controller's hold, load the driver, bind it alone |
| Internal drive on (read-only) | The backup's window: a udev rule in `/run` marks every eMMC disk read-only as it appears, the kernel's `force_ro` is checked on every disk and partition, the write and discard counters are recorded | The same, and our kernel makes every eMMC disk read-only when it is created, by a small patch, and refuses to make it writable again, so no window exists at all |
| Off | Unmount, unbind, check the block devices are gone; for the eMMC, check the counters did not move. If a controller won't let go, the MPC powers off, as the backup does | The same, then the hold goes back on |

The kernel needs: the MMC drivers (as modules on our kernel), `vfat` and `exfat` (both in mainline), and whichever filesystems S0 finds on Akai's side. Our kernel's card set-up must match Akai's (bus speeds, cache): step 1's driver surveys show Akai's MMC settings, and ours start from those.

### The internal drive is read with MPFree's own ext4 reader, never mounted

Mounting Akai's ext4 even read-only can write to it: the kernel replays an unfinished journal at mount time. So MPFree never mounts the eMMC. It opens the partition read-only and reads files with the ext4 reader the creator already uses, the way the workstation tools read the eMMC backup with `debugfs`. The kernel's read-only flag is underneath as a second guard.

- **What it reads:** partition 8 (Akai's Internal drive: projects, samples, kits) and partition 7 (factory content). Partition 8 also holds Akai's writable `/etc` and `/var` layers (`/data/system`); MPFree doesn't list those.
- **Factory content** stays on the MPC: MPFree loads it, never copies it to the stick or anywhere else ([factory content policy](../project/factory-content-policy.md)).
- **If Akai's last shutdown was cut off** (journal not clean), MPFree says the drive may show older files, and still only reads.

### Rules for writing to the SD card (layout A)

- MPFree writes only inside `MPFree/` at the top of the card. Anything else on the card it only reads.
- Before the first write after the card goes in, a read-only check (`fsck.exfat -n` or `fsck.vfat -n`). If the card isn't clean, MPFree uses it read-only and says to check it on a computer. MPFree never repairs or formats a card on its own.
- Each file is written whole to `NAME.part`, synced, then renamed, as on the stick's data partition ([storage spec](../design/storage-spec.md)).
- Mounted `nosuid,nodev,noexec`, as Akai mounts it. The power card's Shut down and Restart unmount the card first. A card taken out while mounted is noted, and MPFree says to put it back or check it.

## Steps

Each step is its own PR with owner blocks, verified before the owner gets them. Each passes only with the boot-area hashes unchanged and 0 sectors written to the eMMC, as every round so far.

0. **Survey (read-only).** With the owner's card in, on Akai's system: how `edisksd` mounts an SD card and with which options, which filesystems Akai's kernel has, whether `fsck.exfat` and `fsck.vfat` are on Akai's system, what it does with a partition it can't read, and what Akai keeps on a card (top-level folder names only, in the report on the owner's computer). From the device tree (block K1.1): the SD node's card-detect and write-protect settings. Small additions to `--drivers` and a new read-only `--sd-card` mode.
1. **SD card, read-only, on Akai's kernel (soft-reboot route).** The Browser gets an "SD card" place. Opening it binds the SD controller alone and mounts the card read-only; leaving it or shutting down unmounts and unbinds. Passes when the card's write counter stays at 0 and the owner loads a sample from it.
2. **SD card, saving to `MPFree/`.** Layout A's rules above. Passes when Akai's system shows MPFree's files afterwards and the card checks clean on a computer. This step changes the [storage spec](../design/storage-spec.md)'s rule zero ("the SD card is never written") and is merged only on the owner's go.
3. **The internal drive, read-only, on Akai's kernel.** The backup's window plus MPFree's ext4 reader; an "MPC internal drive (read-only)" place. Passes with the write and discard counters unchanged and the boot hashes unchanged.
4. **Our own kernel.** The same three on the chain, with the modules held at start and the read-only patch (own-kernel step 2 builds the modules; this step adds the patch and the holds).

Steps 1 to 3 don't wait for our kernel: the soft-reboot route is the default, and the backup already proves the read-only window there. Until step 4 a `--chain` stick keeps both controllers out of its device tree.

## Related pages

- [MPFree's own kernel](own-kernel.md): the kernel these drivers go in.
- [How MPFree boots from the stick](soft-reboot-boot.md): the switch-off at start and the backup's read-only window.
- [The storage spec](../design/storage-spec.md): where MPFree saves today (the stick's partition 4).
- [The MPC One's boot architecture](mpc-one-boot-architecture.md): the eMMC's partitions.
