# The MPFree USB creator: a boot stick with no terminal

**Status:** built, 2026-11-15 ([as built](#as-built-2026-11-15)); not yet run on the reference MPC. The plan was a proposal the owner approved the same day ([decisions](#the-owners-decisions-2026-11-15)). It is a desktop app for Linux, macOS and Windows that turns a musician's own copy of Akai's 3.10.1 update into an MPFree boot stick, and a one-time, terminal-free way to set up the MPC for it. The boot itself is the merged soft-reboot route in [How MPFree boots from the stick](../hardware/soft-reboot-boot.md); the only change to it is an MPC One board check in the hook ([below](#safety-work-before-anyone-else-runs-it)). The musician's steps are in the user manual: [Installing MPFree](../../user/src/getting-started/installing.md).

## As built (2026-11-15)

| Part | Where | What |
|---|---|---|
| The core and command line | [`rust/mpfree-stick`](../../../rust/mpfree-stick) | Reads Akai's update file (the AZ01 container: the xz root filesystem, its SHA-1 or SHA-256 checked) and refuses any update not in its `SUPPORTED` table (Akai build, kernel release, root filesystem size). Copies every file of Akai's root filesystem (ext4 reader) into a **fresh ext2** with MPFree's changes (ext2 writer, both in [`mpfree-diskimage`](../../../rust/mpfree-diskimage)), lays out and writes the stick, reads it back, copies the backup off a stick, switches the setup partition between install and remove ([uninstall](#the-setup-script)), prints the stick's logs with MAC addresses and USB serial numbers hidden. The `mpfree-stick` program (`mpfree-stick help`); the update may be named by its folder (the supported update in it is picked), and the commands that only read a stick, or set its mode, take `--device mpfree` for the one MPFree stick that is in |
| The changes | `rust/mpfree-stick/src/recipe.json`, written by [`tools/mpc-one/creator-recipe.py`](../../../tools/mpc-one/creator-recipe.py) | The recipe is recorded by running `offline-kexec-stick.py`'s own `build_rootfs` (soft-reboot, `--app --app-user --splash`) on a tiny fake image, with placeholders for each stick's values. `test_creator_recipe.py` fails when the builder changes until the recipe is written again, so the two can't drift. What depends on the image (which programs get the stub, `/etc/passwd` and `/etc/group`, root's `!` in `/etc/shadow` with SSH only, folders made when missing) is applied by the same rules in Rust |
| The window | [`rust/mpfree-creator`](../../../rust/mpfree-creator) (its own workspace) | eframe. Finds the update file in Downloads, Desktop or Documents (or takes a dropped file), lists USB sticks, makes the stick, copies the backup, prepares removal; Preferences > Developer options. For each step that touches a disk it runs itself as the command line (`--cli`) with the rights to: `pkexec` on Linux, the administrator password through `osascript` on macOS, an administrator prompt (`Start-Process -Verb RunAs`) on Windows, and reads its progress from a status file |
| USB disks | `rust/mpfree-stick/src/disks.rs` | Only USB disks, never the one the computer runs from, 512-byte sectors only. Linux: `/sys/block`, `umount`, an exclusive open (**tested**). macOS: `diskutil`, `unmountDisk`, `/dev/rdiskN` with whole-sector I/O. Windows: `Get-Disk` (USB, not boot or system), each volume locked and dismounted (`FSCTL_LOCK_VOLUME`, `FSCTL_DISMOUNT_VOLUME`), `\\.\PhysicalDriveN`. macOS and Windows are compile- and lint-checked in CI, **not yet tried on a stick** |
| The stick's own setup | [`tools/mpc-one/stick/`](../../../tools/mpc-one/stick) | `autorun.sh`, `mode.txt` and `README.txt` on the setup partition (with `hook-install.sh` and `hook-remove.sh` byte for byte); `setup-backup.sh` and `setup-format.sh` with their units on partition 1. Tested against fake kernels in `test_stick_setup.py` |
| The first-start setup | [`rust/mpfree-app/src/setup.rs`](../../../rust/mpfree-app/src/setup.rs) | The wizard in the pad player ([below](#the-first-start-wizard-backup-kits-and-samples)) |

The stick:

| Partition | GPT name | What |
|---|---|---|
| 1 | `mpfree-test-root` (`mpfree-test-root-off` while the stick is set to remove MPFree) | Akai's root filesystem with MPFree's changes, a fresh ext2 sized to fit (about 470 MB); the hook mounts it read-only |
| 2 | `mpfree-test-log` | ext2, 32 MiB, `MPFREE-LOG` |
| 3 | `mpfree-test-kits` | ext2, `MPFREE-KITS`: `kits.bin`, MPFree's own kits rendered from the Drums synth |
| 5 | `mpfree-setup` | FAT32, 128 MiB, label `MPFREE` (Akai mounts it at `/media/MPFREE`): the setup files. Placed after partition 3, before 1 GiB |
| 4 | `mpfree-test-data` | From 1 GiB to the end. On a new stick its first MiB is zeroed and starts with `MPFREE-FORMAT-ME <token>`; the token is also in partition 1's `/etc/mpfree-test/format-token`, and `setup-format.sh` makes the ext4 (`MPFREE-DATA`) with Akai's own `mke2fs` at the first start, only when both match. A stick that already has an MPFree partition 4 (name, start at 1 GiB, ext4 label) keeps it |

Each stick gets its own partition GUIDs (the stick's services find partitions 2 to 4 by them), host key (with SSH only) and format token. `/etc/mpfree-test/made-by.txt` records the creator version, Akai build and whether SSH is on; `mpfree-stick inspect` shows it.

Checks: `cargo test -p mpfree-stick` makes whole sticks from a made-up update (a fake Akai root filesystem in an AZ01 file) and reads every partition back, with `e2fsck -fn` where it is installed; writes a 16 GB sparse "stick" twice and checks partition 4 is kept; copies a backup off and refuses a damaged one; switches to remove and back and checks the partition table returns byte for byte; runs every command of the program. The Python tests cover the stick's own scripts, and `test_boot_hook.py` checks that the hook passes over a stick set to remove.

For the end-to-end test on the reference MPC, [`compare-emmc-backups.py`](../../../tools/mpc-one/compare-emmc-backups.py) compares the wizard's backup, copied off the stick, with an SSH backup, region by region ([survey tools](../guides/survey-tools.md#other-tools)).

## The goal

An average musician who wants to try MPFree should get from "my MPC runs 3.10.1 and I have Akai's update file" to "MPFree starts when the stick is in" without a terminal, SSH, keys, scripts or a network cable. The project never hosts or bundles any Akai file.

Out of scope: updating the MPC to 3.10.1. The musician does that first with Akai's own instructions and keeps the update file Akai gave them. The creator starts from there.

## The owner's decisions (2026-11-15)

1. Switching developer mode on once and tapping **Yes** is acceptable as the one step on the MPC.
2. **One stick**, with the setup partition on it. No separate setup stick.
3. Drum kits for creator-made sticks: **both** kits rendered from MPFree's Drums synth (the default) and a few CC0 sample kits with their sources recorded ([below](#what-ships-in-the-app)). As built: the Drums-synth kits; the CC0 kits still have to be chosen.
4. GUI: **egui**, or whatever is simplest; the UI is a small wizard. Bundling e2fsprogs is fine. (As built, nothing is bundled: the stick's filesystems are written in Rust, and partition 4's ext4 is made on the MPC.)
5. Signing: the owner would rather not pay for an Apple Developer ID or a Windows certificate. The free route is in [Releases](#releases); still open.
6. **MPC One (Gen 1) only** for the first release. The app and the manual say plainly that it has only been tested on the MPC One (Gen 1). The setup script and the hook still refuse other boards, because the switch-off service names the MPC One's controller addresses.
7. Legal review of Akai's terms is the owner's to handle; this plan doesn't track it.
8. **Developer SSH stays available**, off by default, behind one checkbox in the app's Preferences under Developer options ([below](#developer-ssh)). The user manual gets clear step-by-step instructions for setting it up.
9. **A full eMMC backup is made automatically, and the musician gets Akai's drum kits and samples with no effort** (owner, 2026-11-15 19:39Z). **Both happen in MPFree's own first-start setup wizard**, not in Akai's script runner (owner, 19:44Z), so Akai's app never has to run a long process. Akai's script only installs the hook. See [the first-start wizard](#the-first-start-wizard-backup-kits-and-samples). The owner approved the read-only eMMC window this needs (19:46Z), on condition that it is tested on the reference MPC first.

## What the musician does (recommended flow)

| Where | Step | Typing |
|---|---|---|
| Computer | Install and open **MPFree USB Creator**. Drop in Akai's 3.10.1 update file (the same one they updated the MPC with). The app checks it is exactly a supported file and says so in plain words. | none |
| Computer | Plug in a USB stick (32 GB or larger: the backup takes about 7.3 GiB and Akai's samples up to 4 GiB). The app lists only removable USB disks, with their name and size. Pick it, click **Create**, approve the system's password prompt. The app builds the stick, writes it and reads it back. | none |
| MPC, once | Plug the stick in and switch on. Switch on developer mode (Menu > Preferences > Enhancements: the developer panel; then SHIFT + MENU > Developer). The MPC asks whether to run the stick's setup script: tap **Yes**. A few seconds later the MPC restarts by itself into MPFree. | none |
| MPC, first start | While the stick's data partition is made (seconds), the startup screen stays up with a card saying so ([below](#progress-the-musician-sees)). Then MPFree's **setup wizard** opens: "Backing up your MPC, so you can always go back", "Checking the backup on the stick", then "Extracting Akai drum kits and samples", with one progress bar for all of it and the time left, for 20 to 30 minutes on a USB 2 stick. Then the pad player opens with the MPC's own kits and samples (and MPFree's own kits). The last page of the wizard reminds the musician to switch developer mode off the next time they use Akai's MPC (the stick out, then the same menu). | none |
| Computer, next time | The **Copy the backup** tab: the app copies the newest backup off partition 4 into a folder the musician chooses (default: MPFree backups in their home folder) and checks it against its SHA256SUMS. The stick keeps its copy (as built, the computer never writes ext4). | none |
| From then on | Stick in at power-on: MPFree. Stick out: Akai's MPC, exactly as before. | none |

The one tap on the MPC is the owner's consent, which the [installer design](usb-installer.md) asks for anyway (`SHOW_DEVICE_CONSENT`).

## What today's process needs, and what the creator replaces

Today's owner sequence is blocks 4.1 to 4.5 of [the stick auto-boot record](../hardware/stick-autoboot.md#blocks-for-the-owner). Each step, checked against the code on main (d1fb37b):

| Step today | Needs SSH or developer mode? | Needs Akai's files? | In the creator |
|---|---|---|---|
| Unpack the update (`offline-update-extract.py`) | No | Yes: the user's update file | The app does it on the computer, from the user's file |
| Copy `/boot` from the image (`offline-rootfs-survey.sh`) | No | Yes: from the same file | The app does it |
| `collect-over-ssh.sh --kexec-preflight` for `live.dtb` | **Yes** | No | **Not needed.** `offline-kexec-stick.py` stops without it, but it only feeds `kexec.dtb`, which the soft-reboot route never uses. The soft-reboot check needs only the kernel release, and on an installed 3.10.1 that is the image's own. Needs a soft-reboot-only build mode |
| eMMC backup (`collect-over-ssh.sh --backup`, `host_emmc_backup.py`) | **Yes** | No | **Moved into MPFree's first-start wizard**: it copies the eMMC to the stick through a short read-only window, with the same checks and stricter ones ([below](#the-first-start-wizard-backup-kits-and-samples)) |
| Drum kits (`offline-factory-kit.py`) | Only through the backup | Yes: Akai's factory sounds, from the owner's own MPC | **Made for the musician** by the same wizard, from the backup on their own stick, together with Akai's samples. Akai's sounds never ship; MPFree's own kits are there as well ([below](#what-ships-in-the-app)) |
| Your SSH key for port 2222 (`MPC_KEY`) | Only to use it | No | Off by default; one checkbox in Preferences > Developer options adds it ([below](#developer-ssh)) |
| Build tools: `debugfs`, `mke2fs`, `e2fsck`, `sfdisk`, `ssh-keygen` | No | No | Linux tools today; the app must bring them or replace them on all three systems |
| `write-test-stick.py` (Linux, `lsblk`, sudo) | No | No | A writer per operating system, inside the app |
| `--devmode-trust` (host key, [Block Q1](../hardware/firmware-3.10.1-qualification.md)) and `--hook-install` over port 22 | **Yes** | No | **Replaced** by the setup script on the stick, run by Akai's script runner. No SSH at all, so no host key to trust |
| `--hook-report` over SSH | **Yes** | No | The setup script and the stick's log partition keep the notes; MPFree's first screen and the app show them |

So the stick build already depends only on the user's own update file, apart from the preflight (not needed). The hook install is the only step that needs Akai's system; the setup script on the stick does it in seconds. The backup and the factory sounds are done by MPFree itself, once it is running.

## Installing the hook without a terminal

**Yes, it can be done with no terminal: by the stick itself, through Akai's own script runner.** On 3.10.1, with developer mode on, Akai's app offers to run `autorun.sh` from a USB drive and runs it as root after one tap. This was tested on the reference MPC on 2026-11-15 ([record](../hardware/stick-autoboot.md#what-the-script-runner-test-showed-2026-11-15)): the prompt came once Akai's app was up, and the script ran as root through `systemd-run` (a transient `autorun-sh.service`), from a FAT32 stick mounted `noexec` (a shell script is read by `sh`, so `noexec` doesn't stop it). It was rejected as the *boot* trigger because it needs a tap on every boot after Akai's app has drawn. For a **one-time setup** that tap is exactly what we want.

### The setup script

The creator adds a small FAT32 partition, partition 5 (`mpfree-setup`, volume label `MPFREE`), between partitions 3 and 4 on the disk (partition 1 must stay `mpfree-test-root`, as the hook requires). It holds:

- `autorun.sh`: a thin wrapper that runs the steps below with `sh` and writes everything it prints to `MPFREE-SETUP/setup-log.txt`;
- `hook-install.sh`, byte for byte as on main, so the files it writes and its undo stay exactly the ones already tested;
- `mode.txt`: `install` or `remove`, set by the app (so the same stick can uninstall; see below), and `removed` once the script has removed the hook.

The wrapper runs, in this order, and stops at the first failure, writing why:

1. **Checks** (read-only): the board is an MPC One (`/proc/device-tree/model` and `compatible`), the kernel release equals the stick's `/boot/mpfree-softreboot/release`, and no firmware updater is running.
2. **The hook install**: `hook-install.sh` writes its three files on `/data` and runs the hook's own read-only check of the stick, ending in a `RESULT:` line. This is the only change to the MPC.
3. **The restart**: once no firmware updater is running and nothing is left at `/run/nextroot` (where a plain reboot would turn into a soft-reboot), `systemctl reboot`. With the stick in, MPFree starts, and its setup wizard takes over.

The whole script takes seconds, so nothing depends on how Akai's app treats a long script.

The script never writes the root filesystem, the boot partitions or the U-Boot environment.

**Showing the result.** The MPC restarting into MPFree is the sign the hook went in. If the script stops early, the MPC doesn't restart and stays in Akai's app; `setup-log.txt` says why, and the app shows it when the stick is next plugged into the computer.

**Uninstall.** Taking the stick out is always enough to get Akai's MPC back. To remove the hook's files too, the app's **Take MPFree off my MPC** tab walks through three numbered steps; its **Make it a removal stick** button (`mpfree-stick setup-mode --device DISK remove`) sets `mode.txt` to `remove` and renames partition 1 to `mpfree-test-root-off` in the partition table. The hook looks only for a partition named `mpfree-test-root`, so with the hook still on the MPC, the musician can switch on with the stick in and Akai's system starts; no change to the hook. The next run of the setup script (developer mode on, tap Yes) runs `hook-remove.sh`, which deletes only files carrying the hook's marker line, writes `removed` in `mode.txt`, and restarts. If Akai's app offers the script again after the restart, tapping Yes says there is nothing to do and doesn't restart, so there is no loop. **Make it an MPFree stick again**, in the same tab's "Changed your mind?" panel (`setup-mode ... install`), names partition 1 `mpfree-test-root` again and writes `install`: the partition table returns byte for byte to what it was.

### The first-start wizard: backup, kits and samples

On its first start, MPFree runs a setup wizard before the pad player, with a progress bar. Each step done is recorded in the store (`settings/setup.conf`, owner `setup`: `done=backup,kits,kit folders`, the backup's folder, how many kits and samples; a step this version no longer has, such as round 4's `samples`, is dropped) as it ends, and the try's number before anything runs, so each step runs once even if a later one is cut off; a failed start is tried again at the next, at most three times, and the pad player plays meanwhile. The store opens before the wizard, so a new partition 4 is marked as MPFree's (`FORMAT`) before the backup or the library is written there ([storage spec](storage-spec.md#layout-of-the-partition)); the third creator stick wrote them first, its store refused the folder, and so nothing was saved and BROWSE had no library. When the store names no backup, the newest finished one on partition 4 is used (a folder the service has named, with its `SHA256SUMS` and `MANIFEST.txt`; a half copy keeps `.part`), so a start whose record was lost doesn't back the MPC up again. Without a store that saves, the wizard doesn't run, since nothing it did would be kept. A rewritten stick keeps partition 4, so nothing runs again: that is the creator's **Update MPFree on this stick** ([below](#updating-a-stick)). A step added in a later version (the MPC's keygroup instruments, after #382) runs alone on a stick set up before, from the backup it already has. It runs only on sticks that have the backup service (the creator's).

**1. "Backing up your MPC": a read-only eMMC window.** Every start of MPFree switches the eMMC and SD card off before udev or any mount ([the switch-off](../hardware/soft-reboot-boot.md#switching-off-the-internal-storage)); that stays exactly as it is. The wizard then asks a small root service, `mpfree-setup-backup.service`, to open a short window. The pad player never does it itself: with `--app-user` it isn't root, and anything that needs root goes in its own unit ([safety rules](../../agents/README.md)). The service:

1. checks that no `mmcblk*` device exists, that every Akai unit that can touch a disk is still masked, and that partition 4 has room, after removing any half copy (`backup/mpc-one-emmc-<UTC>.part/`, never a link) that an earlier try left when the power went;
2. binds **only the eMMC controller** (`ff0f0000`) again; the SD card stays off. The stick's udev rule for `mmcblk*` devices already sets `SYSTEMD_READY=0` and `UDISKS_IGNORE=1`. A second rule, in `/run` for the window only, writes `1` to each eMMC disk's `force_ro` file as it appears (the user area, `boot0` and `boot1`), and the script writes it again itself: the MMC driver then marks the whole disk read-only, every partition with it. Akai's system has no `blockdev` (BusyBox without that applet), so the window uses the kernel's own switch; the first creator stick stopped here on 2026-11-15;
3. checks that the whole disk and every partition read as read-only (`ro` 1), and every disk's `force_ro` is 1, or switches it off and stops;
4. reads every write and discard counter of the disk and its partitions;
5. copies the user area (7.28 GiB), `boot0` and `boot1` (4 MiB each) with MPFree's own `mpfree copy-image` ([`imagecopy.rs`](../../../rust/mpfree-app/src/imagecopy.rs): the disk opened read-only, 1 MiB blocks straight to a file made new, and every MiB of zeros left as a hole) to `backup/mpc-one-emmc-<UTC>.part/` on partition 4, while `sha256sum` hashes the same disk from the eMMC beside it, after checking the disk layout and eMMC model as `host_emmc_backup.py` does (renamed to `backup/mpc-one-emmc-<UTC>/` when verified; root's, readable by the pad player's group). The eMMC reads at about 40 MB/s, faster than the stick writes, so the hash costs no time. While it copies, `vm.dirty_bytes` is 32 MiB (RAM only, put back afterwards, also on failure): the copy runs at most that far ahead of the stick, so the file's size, which the progress shows, is what the stick holds, and there is no long flush at the end. Every 30 s the kernel log gets the pace: MiB copied, KB/s, what the block layer has read from the eMMC and written to partition 4 and to the whole stick, how busy the stick was, and what waits in memory, so a slow side shows which it is. Before the copy it logs the stick's USB speed and largest transfer, and after each disk how much the copy wrote and how much it left as holes;
6. reads the counters again: **no write or discard on any partition**, p7 and p8 included. This is stricter than today's backup, where Akai's system has p7 and p8 mounted read-write; here nothing has them mounted, so the backup is a clean, quiet copy;
7. unbinds the controller, checks that no `mmcblk*` device is left and removes stale links. If the controller can't be switched off again, the MPC powers off, as the startup switch-off does;
8. drops the page cache, reads the backup back from the stick, hashes each image again and compares it with the eMMC's hash (no `sha256sum -c`: one option fewer to need of Akai's BusyBox). Its progress is the stick's block layer read counter, against what the copy wrote (a hole reads as zeros without reading the stick).

A stop from systemd (its own timeout, two hours, or a shutdown) mid-copy is caught: the copy and the hash are stopped, `vm.dirty_bytes` put back, the window closed and the half copy deleted.

The eMMC is reachable only during step 5's copy, which is limited by the stick's write speed on the MPC One's USB 2.0 port (about 7.5 MB/s measured on the reference MPC): roughly 10 to 20 minutes, plus a few to read it back. Any failure leaves the eMMC switched off; the wizard says what happened, and the pad player still works.

**Why no `tee`.** The second creator stick (2026-11-16) copied at about 80 KB/s: "0.1 of 7.8 GB" after 20 minutes. Its copy ran `dd | tee FILE | sha256sum`, and Akai's BusyBox `tee` is built without block I/O: it moves the stream with `getchar` and `putc` on unbuffered streams, one `write` per byte for each output. The same pattern measured 260 times slower than block I/O on a workstation; on the MPC's RT kernel that lands at the rate seen. The eMMC itself had come up at its normal 50 MHz DDR, and read at 40.5 MB/s in the earlier SSH backup.

**Why holes.** The third creator stick (2026-11-16) still took 30 to 40 minutes to copy: `dd` wrote all 7.8 GB to a stick that writes about 7.5 MB/s on the MPC, and Akai's `dd` has no `conv=sparse`. About half of the MPC One's eMMC is unused (the root filesystem 382 of 500 MiB, the content partition 3.0 of 3.9 GiB, the data partition 201 MiB of 2.7 GiB). `mpfree copy-image` checks each MiB and, when it is all zeros, only grows the file's size instead of writing it. The file reads back exactly as the disk, so its SHA-256 is the disk's, and the computer's copy and the kits import read it as before (`mpfree-diskimage` reads holes as zeros). How much it saves depends on how much of the unused space reads as zeros: space that never held a file does; a deleted file's blocks may not. The log says what each copy wrote. The program is MPFree's own, on the stick (`/etc/mpfree-test/mpfree`), so it is not in `NEEDS`; the service stops first if it is missing. It runs before anything else in `main`, opens the source with `File::open` only, and creates the file with `create_new`, so it never overwrites a file or follows a link.

**Other writes on the same stick during the copy.** Two other writers shared the stick with the third stick's copy, and both are cut down: the log service rewrote its copies of the logs every 10 s on a log partition mounted with `sync`, so each of about a hundred small writes waited for the stick (now mounted without it; the `sync` at the end of each round writes them at once); and the kernel zeroes partition 4's inode tables in the background after its first mount (ext4lazyinit), about 0.95 GB on a 64 GB stick at mke2fs's default of one inode per 16 KiB, now about 120 MB at one per 128 KiB (`-i 131072`: still over 100,000 files on a 16 GB stick). The pace lines show both: partition 4's writes beyond the copy's are the inode tables, and the whole stick's beyond partition 4's are the logs.

**2. "Extracting Akai drum kits and samples".** From the backup on the stick, never from the eMMC: the wizard reads the content partition (p7) inside `mmcblk0.img` read-only and
- makes drum kits from the factory drum programs, as `offline-factory-kit.py` does today (`.xpm` files of type Drum, their samples converted to the pad player's kit format), saved on partition 4;
- puts Akai's factory samples into an "Akai factory" folder in MPFree's library on partition 4, sorted into folders so they show up in BROWSE ready to use (`setup::akai_layout`):
  - each drum program (a kit) gets a folder of its own right inside Akai factory, named as the program, with every sound its pads play (all instruments, every active layer);
  - each keygroup (or other) program gets `Instruments/<name>`;
  - the sounds in no program go to `Other sounds/<Akai's folder>`, with a demo project's `_[ProjectData]` dropped from the name;
  - MPC Documents (the musician's own projects on the MPC) and macOS's `._` files are left out. A program with the same name and sounds as one before it (a kit in two demo projects) is left out; other names that are taken get " 2", compared without case so the layout also fits a FAT card. The same backup always gives the same layout, so a later step can find each pad's file there (a kit file per folder, once kit files are on main).

  A sound in two kits is in both folders as one file: the second is a hard link (`Library::link_file_unsynced`; a copy where the file system has no links). There are thousands of files (about 3 GB), so each is written without its own sync and the whole lot is synced once at the end (`Library::write_new_unsynced`, `new_folder_unsynced`, then `sync`): a sync per file costs two flushes of the stick, and the third creator stick copied at about 1 MB/s that way. A file kept from a try that was cut off counts only if its size is right; a short one goes to the Trash and is copied again. After 20 samples in a row that can't be put in place, the step stops and says why, instead of trying thousands more on a stick that has stopped taking writes.

  **Sticks set up before (round 4).** Up to round 4 the step was called `samples` and copied Akai's tree as it is: Akai factory held "Expansions" and "MPC Documents", and "Expansions/MPC One" held about 4,000 sounds in one folder, which took minutes to list. The step is now `kit folders`, so a stick that has `samples` done runs it at its next start, from the backup it has. A sound that is already in Akai's own folders is moved into its kit folder (`Library::move_file_unsynced`, a rename: no copy), and what is left of the old folders (MPC Documents with them) goes to the Trash, where Put back undoes it. The log says how many sounds were already there, moved, shared with a kit and copied.

This is `mpfree-factory`'s import (`Content::from_disk`), a Rust port of `offline-factory-kit.py`. The Python tool stays the reference: the same backup must give the same kits. The kits are saved as `library/Kits/Akai factory kits.mpfkits` and loaded at every start before MPFree's own (16 in all); the samples go to `library/Samples/Akai factory/`, sorted as above, all through `mpfree-library`.

**Akai's programs only.** The stick's scripts run on Akai's system, whose BusyBox lacks some applets (`blockdev`) and options (`od -t`, `dd conv=`, and `tee`'s block I/O). Each script names every program it runs in a `NEEDS='...'` line and stops, touching nothing, if one is missing; the creator reads those lines and makes no stick from an update whose system lacks one (`stick::apply`). The tests run each script with only those programs on `PATH`, with an `od` that refuses `-A` and `-t` and a `dd` that refuses `conv=`, `iflag=`, `oflag=`, `ibs=`, `obs=` and `status=`, as the MPC's do. The format step clears its mark through the shell's read-write redirection (`1<>`), which never truncates, since there is no `conv=notrunc`; the second creator stick stopped there.

### Progress the musician sees

The musician waits twice before playing, and each wait says what it is and how far it is:

- **Preparing the stick (seconds).** `setup-format.sh` makes partition 4's filesystem before the pad player can start, with a 64 MiB journal (`-J size=64`: mke2fs zeroes the journal, and its default for a 64 GB stick, 256 MiB, took 37 s of the second stick's first start) and one inode per 128 KiB (`-i 131072`, [above](#the-first-start-wizard-backup-kits-and-samples)). It reports in `/run/mpfree-setup/format-status`: `state` (`checking`, `running`, `done`, `none`, `failed`), `done` (bytes the block layer has written to the partition since mke2fs started) and `total` (64 MiB plus 1/8192 of the partition; mke2fs writes the journal and about 5 MiB of metadata for 61 GB). The creator adds a drop-in to the splash unit (`mpfree-test-splash.service.d/mpfree-setup.conf`, `MPFREE_SETUP_STATUS`), so `mpfree-startup` stays after its scene and draws the pad player's loading card over the last frame, "Preparing the stick for its first start", with MB written and time left, until the state is final (`mpfree_startup::setup`). On a later start the state is `none` within a few seconds, before the pad player could start anyway, so the startup screen ends as before.
- **The wizard (20 to 30 minutes).** One bar for the whole setup (`setup::Estimate`): each stage's share is its expected time, from the bytes it moves (the copy, the read-back, the samples) or the kits it makes, at the rate measured after 10 s, or a first guess before that (12 MB/s copying, holes included; 35 MB/s reading back; 2 kits a second; 3 GB of samples at 5 MB/s). The bar moves with the bytes, never backwards, and the card says the time left for the whole setup once a rate is measured.

The pad player asks for the backup by creating `/run/mpfree-app/setup-backup.request`; `mpfree-setup-backup.path` starts the root service, which removes the request and reports in `/run/mpfree-setup/status` (`key=value` lines) for the progress bar.

**3. Done.** The pad player opens with the MPC's own kits and samples, plus MPFree's own kits. The last page reminds the musician to switch developer mode off.

**The backup belongs on the computer**, not only on the stick. The app's **Copy my MPC's backup** copies the newest `backup/mpc-one-emmc-<UTC>/` off partition 4 (it reads ext4 itself) to a folder the musician can see (MPFree backups in their home folder), into a `.part` folder first, checks every hash, and only then gives it its name. Files are streamed a window of 65,536 blocks at a time (`Ext4::copy_file`), so the 7.3 GiB image copies like any other file; round 4's copy stopped at "files over 4 GiB" because the block map was built whole, and capped there. The limit is now 1 TiB. It doesn't free the space on the stick: that would mean writing ext4 from the computer. A musician who never returns to the computer still has it on the stick.

What the backup is for, said plainly in the app: a full copy of the MPC as it was, including the musician's own projects and settings on p8. It is the starting point for any future recovery, but restoring the boot parts of the eMMC is not possible yet: MaskROM recovery is unproven ([recovery tiers](../research/recovery-and-os-strategy.md)). It holds the musician's data and the unit's identifiers, so it stays on their own stick and computer and is never uploaded. The same goes for the kits and samples.

**Why this order.** It was the owner's call (2026-11-15). Akai's app runs only a seconds-long script, so it doesn't matter whether it would put up with a long one or time it out. MPFree shows real progress on its own screen. And the backup is cleaner, because Akai's system isn't running. The price is the read window: the first moment the eMMC is reachable while MPFree runs. It is read-only and checked by the counters, but it is a change to the boot's safety design. The owner approved it on 2026-11-15, on condition that it is tested on the reference MPC first ([below](#safety-work-before-anyone-else-runs-it)).

### Options considered for the hook install

| Option | Verdict | Why |
|---|---|---|
| **A. Setup script on the stick, run once by Akai's script runner** | **Recommended** | No terminal, no network, no keys. One tap. Uses Akai's documented developer mode. Reuses `hook-install.sh` unchanged |
| B. The app connects to the MPC over the network (developer-mode SSH) | Developer path only | Needs a cable or Wi-Fi, finding the MPC's address, firewall prompts, and Akai's SSH, which takes root with no password until locked. Today's tools already do this for developers |
| C. The developer panel's on-screen terminal, typing one command | Rejected | Still a terminal |
| D. No hook: the script runner starts MPFree on every boot | Rejected by the owner | A tap on every boot, after Akai's app is up |
| E. A unit in Akai's root filesystem, or U-Boot booting USB | Forbidden | Root filesystem and U-Boot writes ([safety rules](../../agents/README.md)) |

### Developer mode afterwards

While developer mode is on, the MPC runs root SSH with no password on any network it is connected to ([compatibility page](../hardware/firmware-3.10.1-compatibility.md)). So it should be switched off after the setup. Akai's `disable.sh` deletes only `/data/root` and `/data/ssh` and restarts; the hook's files live in `/data/system/etc/overlay` and `/data/mpfree-hook`, so they should survive. **That has not been seen on the device yet** and is the first thing to check (below).

Because the setup now ends by restarting into MPFree, MPFree's first screen carries the reminder: next time you use Akai's MPC (stick out), switch developer mode off in the same menu. A smoother option to test: the setup script runs Akai's own `disable.sh` as its last step instead of `systemctl reboot`, which switches developer mode off and restarts in one go. Whether Akai's app then still believes developer mode is on (and turns it back on at the next start) is unknown, so this waits for a test.

## The creator app

### Shape

| Option | Verdict | Why |
|---|---|---|
| **1. One app that builds and writes** | **Recommended** | One download, one window. It can keep the stick's saved settings (partition 4) when rewriting, as `write-test-stick.py` does |
| 2. A builder that makes `stick.img`, written with Raspberry Pi Imager or balenaEtcher | Fallback | Avoids writing our own disk writer, but it is two apps, and those writers erase partition 4 every time |
| 3. The Python tools packaged with a GUI wrapper (PyInstaller) | Prototype only | Fastest reuse, but it drags `debugfs`, `sfdisk` and Python packaging onto macOS and Windows |
| 4. A web page | Not possible | Browsers can't write raw USB disks |

### Technology

A Rust app (ADR-0001), split into a core crate (call it `mpfree-stick`: unpack the update, check its hash, copy and change the root filesystem, lay out the GPT image, verify) with a command-line front end for tests and CI, and a small wizard GUI in `egui`/`eframe` (the owner's choice: pure Rust, one binary per system, no web stack). The project's own `mpfree-ui` draws to a framebuffer, so it isn't used here.

**Editing ext4 on macOS and Windows** is the hardest part, because the builder changes a copy of Akai's ext4 root filesystem with `debugfs -w`. Options:

1. **Bundle e2fsprogs** (`debugfs`, `mke2fs`, `e2fsck`) built for each system and run them as separate programs, keeping the builder's tested command list. They are GPL-2; shipping them as separate programs next to an MIT app is fine with their source offered. Chosen first (owner, 2026-11-15), then not needed:
2. Write the filesystem in Rust. **As built:** not edits to Akai's ext4, but a fresh plain ext2 (no journal, extents or checksums, which Linux's ext4 driver mounts with `ro,noload`) holding every file, link, device, hard link, owner, mode and extended attribute of Akai's, plus the recipe's changes. Partition 4, which must be ext4 and as big as the stick, is made on the MPC with Akai's own `mke2fs` at the first start.
3. Make partition 1 squashfs or erofs, which are simple to write from code. That changes how the hook mounts the stick (`ro,noload` is an ext4 option) and needs Akai's kernel to support it: a hook change, so the owner's call.

The Python builder stays the reference: the recipe is recorded from its own `build_rootfs`, and a test fails when the two differ.

**Writing the stick** follows the device rules in the [installer design](usb-installer.md#product-goal) and `write-test-stick.py`: only whole USB disks the system calls removable, within a size range, nothing mounted from them, shown by name and size, a clear confirm, an exclusive open, a read-back compare, and partition 4 kept. Per system:

| System | How |
|---|---|
| Windows | The writer runs as administrator (UAC prompt); locks and dismounts the stick's volumes, writes `\\.\PhysicalDriveN` in whole sectors |
| macOS | `diskutil unmountDisk`, then the raw device `/dev/rdiskN` opened through the system's authorization prompt |
| Linux | `pkexec` (the desktop's password prompt); `umount` of the stick's partitions, then an exclusive open |

Raspberry Pi Imager (Apache-2.0) has solved these on all three systems and is the reference to read.

### Updating a stick

A newer MPFree goes onto a stick that already has it without a new backup (owner, 2026-11-16): writing a stick keeps partition 4, which holds the backup, the setup's done steps, the settings, the library and the Akai kits and samples. The first start then finds every step done and opens the pad player straight away; a step a newer version adds runs alone, from the backup the stick has. A kits file kept from before is counted from its header, so the log's "done: N Akai kits" stays true (the fifth stick said 0).

- **Which sticks.** `disks` marks a USB disk that looks like an MPFree stick (`DiskInfo::mpfree`), without administrator rights: on Linux a partition named `mpfree-test-root` or `mpfree-test-root-off`, on macOS and Windows a volume labelled `MPFREE` (partition 5). It is only a guess for the window's wording; what is kept is decided by `write` itself.
- **The window.** For such a stick, step 3 reads **Update MPFree on this stick**, with two choices: **Keep my backup, settings and files** (the default; button **Update MPFree**, and a confirm without the erase checkbox, which says what is kept) and **Erase everything and start over** (`--fresh-data`; the erase confirm names the backup). The finish page then says **MPFree is updated**: put the stick back and turn the MPC on, with the new-stick steps folded away for an MPC that has lost the start-up helper (whether an Akai update removes it is still open, [below](#still-open)). It tells the two apart by the `write` log's note that partition 4 is kept (`stick::KEPT_NOTE`).
- **Never erasing the backup by surprise.** Keeping passes `--keep-data`: if partition 4 holds an ext2/3/4 filesystem that `existing_data` won't keep (another place or label, from another version), `write` stops before writing anything and says to copy the backup off first (`stick::holds_data`). A partition 4 that never started (still marked) holds nothing, so it is written.
- **The update file.** The stick's system is built from Akai's update file, so step 1 still needs it.

### What ships in the app

- MPFree's own programs for the stick (pad player `mpfree`, startup screen `mpfree-startup`, screen log `mpfree-screen-log`; `stick::PROGRAM_FILES`), built for ARM by CI and pinned by hash. The startup screen was missing from the first creator sticks (2026-11-15: the pad player showed its own star backdrop instead); it is now in the recipe as the builder's `--splash` adds it, and a programs folder without it is refused. Bundling them means one app version is one tested set and works offline.
- A table of supported Akai updates, deny by default. As built it names 3.10.1 by Akai build (`SNAPSHOT-20261107130239`), kernel release (`6.18.44-az01-2026-08-10-rt6`) and the root filesystem's size (453,051,392 bytes), and the record's own SHA-1 is checked; the whole file's SHA-256 is to be added once read from the owner's copy. A hash is not Akai's content.
- **MPFree's own drum kits**, with a provenance list. The pad player can't start without `kits.bin` today, and the kits on the reference stick are Akai's factory sounds, copied out of the owner's eMMC backup by `offline-factory-kit.py`. A musician has no such backup, and Akai's sounds can't ship. The choices (the owner chose **both**, 2026-11-15):
  - **Rendered from MPFree's Drums synth.** CI plays each pad of a few Drums kits once and writes the hits into `kits.bin` (the format in `rust/mpfree-app/src/kit.rs` is plain 44.1 kHz audio, so nothing in the pad player changes). Entirely MPFree's own, nothing to license, but synthetic in character (808/909-style).
  - **CC0 samples.** Public-domain recordings of real drums. More realistic, but each file needs its source and licence recorded, and someone has to choose them.
  - **Both** (chosen): synthetic kits as the default, plus a few CC0 kits.
  - On top of these, **the MPC's own factory kits and samples**, made automatically from the musician's eMMC backup by MPFree's first-start wizard ([above](#the-first-start-wizard-backup-kits-and-samples)). They never ship with the app.

### Developer SSH

The app's **Preferences > Developer options** has one checkbox, **Developer SSH** (off by default). When ticked, the app takes the user's SSH public key file (filled in from `~/.ssh` when there is one), or makes a key pair for them (`~/.ssh/mpfree_ed25519`, never over an existing file), and the stick gets today's key-only root SSH on port 2222 with an ed25519 host key made for that stick; root's locked `!` becomes `*` (key login only) on that stick alone. The app shows the host key's fingerprint and the `ssh` line. Without the checkbox the stick has no SSH server, Ethernet service or keys at all. The user manual's [Installing MPFree](../../user/src/getting-started/installing.md#developer-ssh-for-developers) has the steps.

### Releases

GitHub Releases, built by GitHub Actions: a Linux AppImage, a Windows installer and a macOS app, each with a SHA-256 and a GitHub build attestation (free; proves the file was built from this repository). A user manual page, "Installing MPFree", is written when the app exists.

Signing, by system, if nothing is paid for:

| System | Without paying | What the musician sees | Paid option |
|---|---|---|---|
| Linux | Nothing needed | Nothing; the AppImage runs after marking it executable (most file managers do it with one click) | None needed |
| Windows | Unsigned at first; apply to **SignPath Foundation**, which signs open-source projects for free | Unsigned: a blue "Windows protected your PC" box on first run; **More info**, then **Run anyway**. Signed: that box fades out as downloads build up reputation | A certificate or Azure Trusted Signing, roughly US$10 a month and up |
| macOS | Unsigned (ad-hoc signed, so it runs on Apple Silicon) | macOS refuses the first open. Since macOS 15 the right-click > Open trick is gone: the user opens **System Settings > Privacy & Security**, clicks **Open Anyway** and enters their password, once | Apple Developer ID, US$99 a year: no warning at all |

The creator is opened once per MPFree update, so a one-time click-through is tolerable if the manual shows it with pictures. Recommendation: launch free (Linux as is, Windows through SignPath once accepted, macOS unsigned with the Open Anyway steps), and pay for the Apple ID only if beta testers stumble on macOS.

## Akai firmware and legal limits

The owner handles the legal review; this section only records the technical facts that bear on it.

- **Nothing of Akai's is distributed.** The app reads the update file the musician already downloaded from Akai, on their own computer. It links to Akai's download page and never downloads or hosts the file itself, so the musician accepts Akai's terms directly. This matches the [factory content policy](../project/factory-content-policy.md).
- **The finished stick holds a changed copy of Akai's root filesystem.** It is the musician's personal copy, and the app says so: don't share stick images. The project never publishes a prebuilt image.
- **No factory sounds** on a creator-made stick. A later, optional importer of the owner's own content stays local and needs the legal review the policy already asks for.
- **Nothing is circumvented.** The route uses Akai's documented developer mode and script runner and Akai's own running kernel. It doesn't touch the eMMC boot partitions, the verified root filesystem, LoadPin, or any licensing or activation.
- Points for the owner's review: Akai's terms on changing a copy of the firmware for personal use and on studying its format; using Akai's product names (plain "for the Akai MPC One" wording, no logos, a "not affiliated with Akai or inMusic" notice); and the GPL source offer for the bundled e2fsprogs.

## Safety work before anyone else runs it

| Item | Kind | Why |
|---|---|---|
| A board check in the hook and the setup script: MPC One only | **Hook change, in the creator's pull request for the owner's go**: `hook-install.sh` and the hook's `prepare` stop unless `/proc/device-tree/compatible` names `inmusic,acva`; `autorun.sh` checks the same | Akai's Gen1 image covers eight boards, and the switch-off service names the MPC One's controller addresses. Before, the hook checked only the kernel release. On a board that fails it, the hook stops and Akai's system starts |
| Does the hook survive switching developer mode off? | Read-only check on the device (`--hook-report` after the restart) | The flow depends on it |
| Does the script runner offer `autorun.sh` from a FAT32 partition that is partition 5 of a GPT stick with ext4 partitions? | Test with a harmless script | The one-stick design depends on it |
| **The read-only eMMC window**: does the eMMC controller bind again after the startup switch-off on this kernel, come up read-only before anything opens it, and unbind cleanly? Do the write counters stay still? Are any Akai units started by the new device events (the build report's udev list)? | Approved by the owner (2026-11-15); first a test on the reference MPC under MPFree's stick (RAM-only driver binding, no writes; the counters prove it), before any musician runs it | The backup and the kits depend on it. If the window can't be made safe: the backup goes back into Akai's script runner |
| The window's service, rehearsed against a fake kernel like `internal-off.sh`'s tests, through every way it can stop | **Done**: `test_stick_setup.py` | It is the only code that reaches the eMMC while MPFree runs |
| What Akai's system does with the stick's partitions while it sits in during setup (mounts, `no-reserved-blocks@` tune2fs, writes to partition 4) | Read-only, from a stock-boot journal | During setup the stick stays in under Akai's app for a minute or more |
| Does `/data/system/etc/overlay` survive an Akai update? | Watch on the next Akai update | Either way the hook refuses the stick (release check) and Akai's MPC starts; the app then needs a new supported image |
| A soft-reboot-only build (no `live.dtb`), SSH only when asked for, MPFree's own kits, the setup partition | **Done** in `mpfree-stick` (the Python builder unchanged) | The creator's build |
| The factory-kit import in Rust (from `offline-factory-kit.py`), with the Python tool as the reference: the same backup must give the same kits | **Done** in `mpfree-factory`; the side-by-side check on the owner's backup is still to run | Used by MPFree on the MPC |

## Plan in stages

1. **Prove the flow on the reference MPC.** Built (2026-11-15); the run is next: make a stick with the creator on Linux, developer mode on, tap Yes, MPFree starts, the backup runs; compare it with the owner's last SSH backup; developer mode off.
2. **The first-start wizard**: the factory-kit and sample import in Rust, with its progress screens. Built.
3. **The Rust core and command line**, checked against the Python builder. Built.
4. **The GUI and the writers**, with the backup copy to the computer. Built: Linux tested in CI and here; Windows and macOS compile-checked, to be tried on real machines.
5. **Signing, release builds, CC0 kits, and a small beta** with a few musicians. The manual page is written.

## Still open

1. Signing: launch free as in [Releases](#releases), or pay for an Apple Developer ID?
2. The board check in the hook (a hook change) is in the creator's pull request: the owner's go.
3. Not seen on the device yet: the script runner offering `autorun.sh` from partition 5 of a GPT stick; the hook surviving developer mode off; the read window.
4. The CC0 kits, and the release workflow (AppImage, Windows and macOS builds with `stick-programs/` beside them).
5. The MPC's keygroup instruments in the first-start setup, as a later step, after #382 (owner, 2026-11-15).
