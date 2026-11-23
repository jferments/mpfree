# The SD card and the MPC's internal drive in MPFree (plan)

**As of 2026-11-23** · **Status:** plan; step S0's read-only survey is built and on `main` (`collect-over-ssh.sh --sd-card`), waiting for the owner's run ([block S0.1](#step-s0-block-for-the-owner)), now in device session 3. Step S1's read-only window is built and tested, and on the creator's stick with its units, doing nothing until the pad player asks ([step S1](#step-s1-the-read-only-window)). The owner chose how the card is shared on 2026-11-17: **a folder on the card as it is** (layout A). · **Asked for by** the owner, 2026-11-17 00:10Z: "eventually I would like for MPFree to be able to utilize the SD card to store samples, etc. and I would like to be able to have read-only access to the eMMC as needed. So these should be something that our kernel has the capability to turn on/off. We need to come up with a plan to safely partition / use the SD card and share it with Akai factory OS."

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

**The owner chose A on 2026-11-17 (00:24Z).** The three ways that were offered:

| | **A. A folder on the card as it is** (recommended) | B. A partition of MPFree's own | C. A second card just for MPFree |
| --- | --- | --- | --- |
| The card | Unchanged: its one FAT32 or exFAT partition, as Akai or a computer made it | Akai's partition made smaller, plus an ext4 partition for MPFree | A card MPFree formats (ext4) |
| What MPFree writes | Only inside one top folder, `MPFree/` | Only its own partition | The whole card |
| Akai's system | Sees MPFree's folder; its samples are ordinary WAV files Akai can load | Mounts MPFree's partition too, read-write, and runs `tune2fs` on it | Mounts it if it is in while Akai runs |
| A computer | Reads everything | Can't read the ext4 partition (Windows, macOS) | Can't read it (Windows, macOS) |
| A power cut while saving | FAT and exFAT keep no journal, so a cut can damage the filesystem, the same risk Akai's own saves carry. MPFree lowers it: whole files written then renamed, synced, the card checked before writing | ext4's journal protects it | ext4's journal protects it |
| Getting there | Nothing to do | Repartitioning a card that has data on it means copying everything off and back (there is no safe exFAT shrink); never done without a typed confirmation | Formatting a card; never done without a typed confirmation |

A keeps one card that works everywhere, with nothing to repartition. B or C can be added later for musicians who want a journal.

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

0. **Survey (read-only).** With the owner's card in, on Akai's system: how `edisksd` mounts an SD card and with which options, which filesystems Akai's kernel has, whether `fsck.exfat` and `fsck.vfat` are on Akai's system, what it does with a partition it can't read (as far as its unit files, udev rules and journal say), and what Akai keeps on a card (the names in the card's top folder, in the report on the owner's computer only). Also the SD node's card-detect and write-protect settings and the bus speed the card runs at, which our kernel's settings start from. **Built 2026-11-17** as [`collect-over-ssh.sh --sd-card`](../guides/survey-tools.md) ([`read-only-sd-card.sh`](../../../tools/mpc-one/read-only-sd-card.sh)): it reads procfs, sysfs, the device tree and udev's records, and never runs `blkid`, `fsck`, `mkfs` or `tune2fs`, opens a block device or mounts. It lists the card's top folder only when the card is FAT or exFAT, whose top folder keeps no dates.
1. **SD card, read-only, on Akai's kernel (soft-reboot route).** The Browser gets an "SD card" place. Opening it binds the SD controller alone and mounts the card read-only; leaving it or shutting down unmounts and unbinds. Passes when the card's write counter stays at 0 and the owner loads a sample from it. **The window is built** (2026-11-23) as [`setup-sd-card.sh`](../../../tools/mpc-one/stick/setup-sd-card.sh), tested against a fake kernel in [`test_sd_card_window.py`](../../../tools/mpc-one/tests/test_sd_card_window.py) ([below](#step-s1-the-read-only-window)). It is on the creator's stick with its units ([below](#on-the-stick)), and does nothing until the pad player asks; the Browser's place comes next, with what S0 finds.
2. **SD card, saving to `MPFree/`.** Layout A's rules above, with what the kernel's FAT and exFAT drivers do ([design](#step-s2-saving-to-the-card-design-not-built)). Passes when Akai's system shows MPFree's files afterwards and the card checks clean on a computer. This step changes the [storage spec](../design/storage-spec.md)'s rule zero ("the SD card is never written") and is merged only on the owner's go.
3. **The internal drive, read-only, on Akai's kernel.** The backup's window plus MPFree's ext4 reader; an "MPC internal drive (read-only)" place. Passes with the write and discard counters unchanged and the boot hashes unchanged ([design](#step-s3-the-internal-drive-read-only-design-not-built)).
4. **Our own kernel.** The same three on the chain, with the modules held at start and the read-only patch (own-kernel step 2 builds the modules; this step adds the patch and the holds).

Steps 1 to 3 don't wait for our kernel: the soft-reboot route is the default, and the backup already proves the read-only window there. Until step 4 a `--chain` stick keeps both controllers out of its device tree.

## Step S1: the read-only window

[`setup-sd-card.sh`](../../../tools/mpc-one/stick/setup-sd-card.sh) follows the backup's eMMC window ([`setup-backup.sh`](../../../tools/mpc-one/stick/setup-backup.sh)): root runs it when the pad player asks (`/run/mpfree-app/sd-card.request`), and it keeps the card open until the pad player asks to close it (`sd-card.close`). It never writes to the card.

1. **Checks first:** the startup switch-off finished, no `mmcblk` device is there, neither the SD nor the eMMC controller has a driver, the first-start backup isn't running and the internal drive isn't open for Akai's kits, and nothing is mounted at `/run/mpfree-sd`. So the eMMC's window and the card's never overlap; if both were asked for at the same moment, each would see the other's disk and close (both mark every `mmcblk` disk read-only).
2. **The window:** the same udev rule in `/run` that marks each `mmcblk` disk read-only as it appears, then only the SD controller (`ff0c0000`) is bound. Every `mmcblk` device must be under that controller's folder in sysfs, and none may be a boot or RPMB area, which only an eMMC has: anything else closes the window at once, and an eMMC disk that stays turns the MPC off. The card is identified by where it sits, never by its name (`mmcblk1` with the eMMC beside it, perhaps `mmcblk0` alone). The disk and each partition must read `ro = 1`, `force_ro = 1`. An empty slot closes the window and says so.
3. **The filesystem:** one partition, or none (a card formatted without a table). Its first sector, read once with `dd` from a disk the kernel already holds read-only into a file in `/run`, must say exFAT (offset 3) or FAT32 (82) or FAT12/16 (54); NTFS and anything else are named and left alone. The same copy says whether the card was **safely removed** last time: Linux marks a FAT filesystem as in use when it mounts it for writing (bit 0 of byte 65 on FAT32, of byte 37 on FAT12/16) and an exFAT one at its first write (bit 1 of VolumeFlags, byte 106; bit 2 there means a disk error was seen), and clears the mark at the unmount, so a mark still set means a cut or a card pulled out while mounted. A read-only mount leaves the mark as it is (Linux 6.18, Akai's kernel: `fat_set_state` in `fs/fat/inode.c` returns at once on a read-only mount, and `exfat_set_vol_flags` in `fs/exfat/super.c` changes nothing on disk). The card still opens, read-only, with `dirty=yes` in its status and a line saying to check it on a computer. Other systems may keep their own mark elsewhere (Windows also uses the FAT's second entry), which this doesn't read. The kernel must already have that filesystem in `/proc/filesystems`: after the soft-reboot no module can load, so if S0 finds `vfat` or `exfat` built as modules, the stick lists them in `preload-modules.txt`.
4. **Mounted read-only** at `/run/mpfree-sd`: `ro,nosuid,nodev,noexec,noatime`, files root's and readable by the pad player's group (`gid=960,fmask=0137,dmask=0027`).
5. **While open,** once a second: the card's write and discard counters (stat fields 5-7 and 12-14, disk and partitions) must not move and the disk must stay read-only; the card taken out closes the window. The pad player moves a counter in `/run/mpfree-app/sd-card.alive` while it uses the card, as it does for the content window; once the counter stands still for 120 s (the pad player stopped), the window closes as if asked.
6. **Closing:** the counters again, and the kernel's warnings. On a 6.x kernel the block layer lets a write to a read-only disk through and only warns, once per disk (`Trying to write to read-only block-device mmcblk1p1`). Nothing can open a read-only disk for writing or mount it read-write, so a write would have to come from inside the kernel, but the window counts those warnings too, and one that names an `mmcblk` disk fails it. Then it unmounts (after 10 s of the pad player still holding a file, lazily: the card is read-only), unbinds the controller, waits for every `mmcblk` device to go, and removes the rule, stale `/dev/disk` links and the mount point. A controller that won't let go turns the MPC off, as the backup's does.

Its state is in `/run/mpfree-setup/sd-card-status` (`checking`, `opening`, `open` with the card, filesystem, size, mount point and whether it was left marked as in use, `closing`, `closed`, `none`, `failed` with why). Every status gives back, as `ask`, the hex word the pad player wrote in its request (at most 32 hex digits, nothing else from the file), so the pad player knows an answer is to its own request and not an earlier opening's. Its notes are in `sd-card-log.txt` there and the kernel log. A stop from the system closes it cleanly.

**Tests:** [`test_sd_card_window.py`](../../../tools/mpc-one/tests/test_sd_card_window.py) runs it under dash with only its `NEEDS` programs, against a fake kernel whose sysfs answers the bind and unbind as the controller and udev do (the `/sys/block` entry a link into the controller's folder, `ro` following `force_ro`). It covers FAT32, FAT16 on a card without a table and exFAT; each with its in-use mark set and clear (and exFAT's disk-error bit, and other bits that don't count); an empty slot; NTFS, ext4, two partitions and a kernel without exFAT, each left unmounted; each check before the window; a boot area under the SD controller, an eMMC disk (the MPC turns off), a card that stays writable, udev missing the rule, a failed mount; writes while open, a kernel warning (and that another disk's, or one from before, doesn't count), the card taken out, a stop from the system, the pad player slow to let go, the pad player's counter standing still (and one left from an earlier opening), and a controller that won't let go. Static checks: the one `dd` read, the one read-only `mount`, no other disk tool, and sysfs written only for `force_ro`, the bind and the unbind.

### On the stick

The creator puts the script on partition 1 with two units, as it does the backup's ([recipe](../../../rust/mpfree-stick/src/recipe.rs), `SETUP_FILES`):

- `mpfree-setup-sd-card.path` waits, from `sysinit.target`, for `/run/mpfree-app/sd-card.request` and starts the service. The backup's path unit never starts it, nor it the backup.
- `mpfree-setup-sd-card.service` runs the script once per opening (`Type=oneshot`, no time limit: the card stays open as long as the pad player uses it), only after the startup switch-off (`Requires=mpfree-test-internal-off.service`). It stops at shutdown (`Conflicts=shutdown.target`), which closes the window cleanly first.

Nothing opens the card until the pad player asks, so a stick with these units starts exactly as before. The creator makes a stick only from an update whose root has every program the stick's scripts name in `NEEDS` (in `/usr/bin` or `/usr/sbin`); this script adds `mount`, `umount`, `dmesg` and `rmdir` to the backup's and the formatter's. **Not yet checked on the owner's update:** a `mpfree-stick build --out` to a file on the workstation checks it before any stick is written, and says which program is missing if one is.

### The pad player's side

[`mpfree-app/src/sd_card.rs`](../../../rust/mpfree-app/src/sd_card.rs) is the pad player's half, not yet used by a screen: the Browser's "SD card" place comes next.

- `SdCard::open` asks with a new word of 32 hex digits in `sd-card.request` (written whole under a hidden name and renamed in) and waits up to 60 s for a status that gives that word back: `open` gives the `Card` (its filesystem, size, mount point, and `dirty` when it wasn't safely removed last time), `none` and `failed` say why (`NotOpened`). A request nobody picked up in time is taken back, so the card never opens later unasked. A stick without the service says it needs making again.
- While the card is open, a thread of its own writes the counter to `sd-card.alive` every 2 s (never through a link: a counter that can't be written lets the window close by itself after 120 s).
- `SdCard::close` asks with `sd-card.close` and waits up to 60 s for the service's last word; dropping an `SdCard` asks without waiting.
- A pad player started again while its earlier window is still open (the status says `open` and its own mounts list the card) takes that window over, counter and all, rather than asking again: a request made while a window is open would open the card again once it closed.

Tests (`sd_card::tests`) run it against a stand-in service thread: open, the counter moving, close; an earlier opening's status (an empty slot, or open without its mount) not taken for this one's; an empty slot and a card it doesn't read; a request nobody answers, taken back; a stick without the service; a window taken over; a drop that closes; the counter and the close never written through a link.

**What S0 settles before the Browser opens it:** whether Akai's kernel has `vfat` and `exfat` built in (or the stick preloads them, with the code pages FAT needs); how Akai's system mounted the card (the options to match); whether the card shows as one partition; and how long a card takes to answer after the bind (10 s allowed).

## Step S2: saving to the card (design, not built)

What the kernel does when MPFree writes to a FAT or exFAT card, from Linux 6.18's own code (Akai's kernel is `6.18.44-az01`), and the rules that follow. Not built: S2 changes the [storage spec](../design/storage-spec.md)'s rule zero ("the SD card is never written"), so it is merged only on the owner's go, and only after S1 has run on the MPC.

**What the kernel does** (`fs/fat/`, `fs/exfat/`, v6.18):

- **The in-use mark.** FAT is marked in use as soon as it is mounted for writing, and exFAT at its first write; both clear it at a clean unmount (`fat_set_state`, `exfat_set_volume_dirty`). exFAT writes its mark with a cache flush and FUA. A cut while mounted leaves the mark, and the next mount warns "Volume was not properly unmounted. Some data may be corrupt. Please run fsck." S1 already reads the mark ([above](#step-s1-the-read-only-window)).
- **fsync reaches the card.** Both drivers' `fsync` write the file and the FAT (or the whole disk's buffers for exFAT) and then flush the card's cache (`blkdev_issue_flush`).
- **A rename to a new name isn't atomic on FAT.** `vfat_rename` adds the new name's entry, points it at the file's clusters, then removes the old entry. A cut between the two leaves two names on one file's clusters (cross-linked), and deleting either later frees clusters the other still uses: real damage, beyond a lost file. exFAT renames in place when the new name needs no more directory entries than the old (`NAME.part` to `NAME` usually does: up to 15 characters per entry), so the change is one entry set, mostly one sector; a longer name takes the same two steps as FAT.
- **Mount options that help:** `dirsync` (directory changes written as they happen, both drivers), `flush` (FAT only: writes data sooner, so less is cached when a card is pulled), `errors=remount-ro` (both drivers' default: an error found while writing stops the writing).

**The rules, from that:**

1. **Opened for writing only when it is safe to.** The card's in-use mark must be clear, and a read-only check must pass first (`fsck.vfat -n` or `fsck.exfat -n` on the disk the kernel still holds read-only, if S0 finds them on Akai's system; else MPFree's own read of the FAT and the folder's entries). Otherwise the card opens read-only and says to check it on a computer. MPFree never repairs a card.
2. **A second window, the same checks.** The read-only window's checks, then `force_ro` is switched off for the card's disk alone, after it is found under the SD controller (the eMMC controller still has no driver), and it is mounted `rw,nosuid,nodev,noexec,noatime,dirsync` (plus `flush` on FAT). The eMMC's controller stays unbound throughout, as in S1.
3. **Only inside `MPFree/`,** as layout A says, each file whole: written to `NAME.part`, `fsync`, renamed, the folder synced. On FAT the rename's moment is the risk above: the in-use mark is set the whole time the card is open for writing, so a cut there is always seen at the next opening, which is read-only until the card is checked. Whether a hidden-attribute publish (write under the final name with the hidden attribute, then clear it: one entry rewritten in place, `FAT_IOCTL_SET_ATTRIBUTES`) is safer on a real card is measured first (below).
4. **Open for writing only while saving.** The pad player opens the write window for a save and closes it straight after, so the card spends seconds marked in use, not the session. A clean close clears the mark; the power card's Shut down and Restart close it first.

**Tested before the MPC:** the [pull test on FAT](../design/disk-streaming.md#wp-on-fat-the-pull-test-for-saving-to-the-sd-card) (`mpfree-disktest --cut-test --fat`, or `--fat-hidden` for the hidden way, built) on a spare FAT32 and a spare exFAT stick on a computer. Each run writes a stream of whole files and is pulled mid-write. MPFree's check then reads the stick without writing, and the computer's own `fsck -n` checks it too. The runs show how often a pull leaves a cross-link or a lost file with each way of publishing. Then on the MPC with the owner's card, with S1's checks around it and the boot-area hashes unchanged.

**What S0 settles for S2:** whether Akai's system has `fsck.vfat` and `fsck.exfat`, and how it mounts a card (if Akai's own system writes to the card with the same driver, its cuts leave the same mark).

## Step S3: the internal drive, read only (design, not built)

The MPC's own **Internal** drive (partition 8, Akai's `/data`, also mounted by Akai's system at `/media/az01-internal`) as a place in the Browser: the musician's projects, samples and kits from Akai's side, read only. Partition 7, the factory content, already reads this way for Akai's kits without a backup (setup-backup.sh's step 3c and [`mpfree-factory`](../../../rust/mpfree-factory/src/import.rs)); S3 does the same for partition 8, for as long as the browser uses it.

**Never mounted.** Mounting ext4, even read-only, replays an unfinished journal ([above](#the-internal-drive-is-read-with-mpfrees-own-ext4-reader-never-mounted)). The pad player opens the partition's device read-only and reads it with [`mpfree_diskimage::ext4`](../../../rust/mpfree-diskimage/src/ext4.rs), which never writes, never replays a journal and bounds every count it reads.

**The window,** a third mode beside the backup's copy and its step 3c, with S1's protocol rather than 3c's (3c ends with a "done" file and an hour's limit; a place stays open while the browser uses it):

- `/run/mpfree-app/internal-drive.request` with a word; every status gives it back as `ask=`, as S1's do.
- The eMMC controller is bound alone, under the backup's udev rule (every `mmcblk` disk read-only as it appears) and its checks: the startup switch-off finished, no other window open (the SD card's included: both windows mark every `mmcblk` disk read-only, and each sees the other's disk and closes), `ro = 1` and `force_ro = 1` on the disk, its partitions and boot areas, the counters recorded.
- Only partition 8's node becomes root's and the pad player's group's, 0440, in `/dev` (RAM), as 3c does for partition 7. Nothing is mounted. The status names the partition and the journal's state (below).
- While open, once a second: the write and discard counters must not move and every disk must stay read-only, or the window fails and closes. The pad player moves `internal-drive.alive` every 2 s; 120 s still, and the window closes as if asked. `internal-drive.close` closes it.
- Closing is the backup's: counters, the node's owner and mode back, unbind, wait for every `mmcblk` device to go, remove the rule. A controller that won't let go turns the MPC off.

Probably one script, `setup-internal-drive.sh`, sharing the backup's window functions, with the same kind of fake-kernel tests as `test_sd_card_window.py`. The creator puts it on the stick with a `.path` and `.service` pair like S1's.

**Akai's journal.** Akai mounts partition 8 read-write. After a clean shutdown its journal is empty. After a cut, ext4 marks it `needs_recovery` (feature_incompat bit `0x4`, which the reader accepts), and the newest changes may be only in the journal. MPFree reads the filesystem as it stands, without them. So when the bit is set, the window's status says `journal=unclean`, the place opens anyway, and the line under the list says "The MPC wasn't shut down cleanly last time: the newest files may be missing here. Start Akai's MPC software once to tidy it." A file whose blocks don't read as a sound, a kit or a project fails to load and says so. Nothing is ever written.

**The place.** The library is built on real folders. The internal drive is an image read through the reader, so it can't be a library folder.

- **Listing:** the place gets its rows from the reader, as the Plugins place gets its rows from the app: `mpfree-library` gains a read-only `Volume` behind `Place::Internal`, with `list(loc)` from `read_dir` and the inodes' sizes and times, and `read(loc)` from `read_file` (up to `MAX_WHOLE_FILE`). `check_writable` refuses `Place::Internal`, as it does the SD card.
- **Hidden:** Akai's own folders are left out of the top folder: `system` (Akai's `/etc` and `/var` layers), `lost+found`, and names starting with a dot. Links and special files are never shown or followed.
- **Load, Audition and Copy** read the bytes through the volume on the library's worker: a sound is read into memory and handed to the loader as bytes (`LoadSample` with the bytes instead of a path), so nothing is copied anywhere first. Copy, then Paste in a library place, writes the bytes with `Library::write_new`.
- **Akai's own files:** the MPC's projects (`.xpj`) and programs (`.xpm`) open with [`mpfree-mpc-import`](../../../rust/mpfree-mpc-import/src/lib.rs) (sounds and patterns only, decisions), reading each sample the program names through the same volume. That is S3's second part.
- **Its look:** the SD card place's (read-only path line, Copy only, no stars). Its own screens go to Jay first, as the SD card's did.

**Unverified, to settle before it is built:**
- Which top folders Akai's browser shows on the Internal drive. A read-only listing of partition 8's top folder from the owner's eMMC backup (on the workstation, with `debugfs -R ls`) answers it without the MPC.
- Whether Akai's partition 8 uses features the reader refuses (`INCOMPAT_KNOWN`). The same backup answers it (`dumpe2fs -h`).
- How long the reader takes to list a folder of a few thousand samples on the eMMC (htree folders are read whole). A host test on an image of that size first.

**Passes when,** on the MPC with Jay's go and a rehearsed block: the place lists the Internal drive, a sound from it loads and plays, Copy and Paste put it in the library, the counters stay unchanged, and the boot hashes are the same before and after.

## Step S0: block for the owner

**Read-only** on the MPC: it reads files over SSH and changes nothing, on the MPC or on the card. Put your SD card in the MPC's slot and switch the MPC on **without the stick**, on Akai's 3.10.1 with developer mode on and its host key recorded, as for [block K1.2](own-kernel.md#step-1-blocks-for-the-owner). It runs from `main`.

**Block S0.1: update, check, the SD card survey, then the zip.**

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && git switch --no-track -C sd-card origin/main && git log --oneline -1 && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_read_only_sd_card.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run sd-card -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --sd-card ; python3 tools/mpc-one/results.py zip
```

It must say `LOCAL-CHECKS-OK`, then end with `SUCCESS: …mpc-one-sd-card-….txt`. Send the zip; it holds the names in your card's top folder, so it goes to this project only. There is nothing to undo; `git switch main` puts the clone back.

**Passes when** the report ends with `END OF SD CARD SURVEY` and has a `card: mmcblk…` line. If it says `nothing from the card is mounted`, Akai's system did not mount the card, which is an answer too.

## Related pages

- [MPFree's own kernel](own-kernel.md): the kernel these drivers go in.
- [How MPFree boots from the stick](soft-reboot-boot.md): the switch-off at start and the backup's read-only window.
- [The storage spec](../design/storage-spec.md): where MPFree saves today (the stick's partition 4).
- [The MPC One's boot architecture](mpc-one-boot-architecture.md): the eMMC's partitions.
