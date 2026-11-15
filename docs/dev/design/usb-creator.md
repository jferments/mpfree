# The MPFree USB creator: a boot stick with no terminal

**Status:** proposal, 2026-11-15; the owner answered most open questions the same day ([decisions](#the-owners-decisions-2026-11-15)). Nothing here is built. It plans a desktop app for Linux, macOS and Windows that turns a musician's own copy of Akai's 3.10.1 update into an MPFree boot stick, and a one-time, terminal-free way to set up the MPC for it. The boot itself is the merged soft-reboot route in [How MPFree boots from the stick](../hardware/soft-reboot-boot.md); this page changes nothing about it.

## The goal

An average musician who wants to try MPFree should get from "my MPC runs 3.10.1 and I have Akai's update file" to "MPFree starts when the stick is in" without a terminal, SSH, keys, scripts or a network cable. The project never hosts or bundles any Akai file.

Out of scope: updating the MPC to 3.10.1. The musician does that first with Akai's own instructions and keeps the update file Akai gave them. The creator starts from there.

## The owner's decisions (2026-11-15)

1. Switching developer mode on once and tapping **Yes** is acceptable as the one step on the MPC.
2. **One stick**, with the setup partition on it. No separate setup stick.
3. Drum kits for creator-made sticks: **both** kits rendered from MPFree's Drums synth (the default) and a few CC0 sample kits with their sources recorded ([below](#what-ships-in-the-app)).
4. GUI: **egui**, or whatever is simplest; the UI is a small wizard. Bundling e2fsprogs is fine.
5. Signing: the owner would rather not pay for an Apple Developer ID or a Windows certificate. The free route is in [Releases](#releases); still open.
6. **MPC One (Gen 1) only** for the first release. The app and the manual say plainly that it has only been tested on the MPC One (Gen 1). The setup script and the hook still refuse other boards, because the switch-off service names the MPC One's controller addresses.
7. Legal review of Akai's terms is the owner's to handle; this plan doesn't track it.
8. **Developer SSH stays available**, off by default, behind one checkbox in the app's Preferences under Developer options ([below](#developer-ssh)). The user manual gets clear step-by-step instructions for setting it up.
9. **The setup makes a full eMMC backup automatically, and the musician gets the MPC's own drum kits with no effort** (owner, 2026-11-15 19:39Z). See [the backup and the MPC's own kits](#the-emmc-backup-and-the-mpcs-own-drum-kits).

## What the musician does (recommended flow)

| Where | Step | Typing |
|---|---|---|
| Computer | Install and open **MPFree USB Creator**. Drop in Akai's 3.10.1 update file (the same one they updated the MPC with). The app checks it is exactly a supported file and says so in plain words. | none |
| Computer | Plug in a USB stick (16 GB or larger; the backup needs about 7.3 GiB of it). The app lists only removable USB disks, with their name and size. Pick it, click **Create**, approve the system's password prompt. The app builds the stick, writes it and reads it back. | none |
| MPC, once | Switch on developer mode (Menu > Preferences > Enhancements, then SHIFT + MENU > Developer). Plug the stick in. The MPC asks whether to run the stick's setup script: tap **Yes**. The setup backs up the whole eMMC onto the stick, then sets the MPC up. That takes roughly 5 to 25 minutes, depending on the stick. When it's done, the MPC restarts by itself into MPFree. | none |
| MPC, first start | MPFree says the setup worked and the backup checked out, and makes drum kits from the MPC's own factory sounds, read from the backup on the stick. Its first screen reminds the musician to switch developer mode off the next time they use Akai's MPC (the stick out, then the same menu). | none |
| Computer, next time | When the stick is next plugged in with the app open, the app copies the backup to the computer, checks it, and frees the space on the stick. | none |
| From then on | Stick in at power-on: MPFree. Stick out: Akai's MPC, exactly as before. | none |

The one tap on the MPC is the owner's consent, which the [installer design](usb-installer.md) asks for anyway (`SHOW_DEVICE_CONSENT`).

## What today's process needs, and what the creator replaces

Today's owner sequence is blocks 4.1 to 4.5 of [the stick auto-boot record](../hardware/stick-autoboot.md#blocks-for-the-owner). Each step, checked against the code on main (d1fb37b):

| Step today | Needs SSH or developer mode? | Needs Akai's files? | In the creator |
|---|---|---|---|
| Unpack the update (`offline-update-extract.py`) | No | Yes: the user's update file | The app does it on the computer, from the user's file |
| Copy `/boot` from the image (`offline-rootfs-survey.sh`) | No | Yes: from the same file | The app does it |
| `collect-over-ssh.sh --kexec-preflight` for `live.dtb` | **Yes** | No | **Not needed.** `offline-kexec-stick.py` stops without it, but it only feeds `kexec.dtb`, which the soft-reboot route never uses. The soft-reboot check needs only the kernel release, and on an installed 3.10.1 that is the image's own. Needs a soft-reboot-only build mode |
| eMMC backup (`collect-over-ssh.sh --backup`, `host_emmc_backup.py`) | **Yes** | No | **Moved into the setup script**: it streams the eMMC to the stick instead of over SSH, with the same checks ([below](#the-emmc-backup-and-the-mpcs-own-drum-kits)) |
| Drum kits (`offline-factory-kit.py`) | Only through the backup | Yes: Akai's factory sounds, from the owner's own MPC | **Made for the musician** from the backup on their own stick, by MPFree at first start. Akai's sounds never ship; MPFree's own kits are there as well ([below](#what-ships-in-the-app)) |
| Your SSH key for port 2222 (`MPC_KEY`) | Only to use it | No | Off by default; one checkbox in Preferences > Developer options adds it ([below](#developer-ssh)) |
| Build tools: `debugfs`, `mke2fs`, `e2fsck`, `sfdisk`, `ssh-keygen` | No | No | Linux tools today; the app must bring them or replace them on all three systems |
| `write-test-stick.py` (Linux, `lsblk`, sudo) | No | No | A writer per operating system, inside the app |
| `--devmode-trust` (host key, [Block Q1](../hardware/firmware-3.10.1-qualification.md)) and `--hook-install` over port 22 | **Yes** | No | **Replaced** by the setup script on the stick, run by Akai's script runner. No SSH at all, so no host key to trust |
| `--hook-report` over SSH | **Yes** | No | The setup script and the stick's log partition keep the notes; MPFree's first screen and the app show them |

So the stick build already depends only on the user's own update file, apart from the preflight (not needed). The backup, the factory kits and the hook install are the steps that need the MPC, and the setup script on the stick does all three.

## Installing the hook without a terminal

**Yes, it can be done with no terminal: by the stick itself, through Akai's own script runner.** On 3.10.1, with developer mode on, Akai's app offers to run `autorun.sh` from a USB drive and runs it as root after one tap. This was tested on the reference MPC on 2026-11-15 ([record](../hardware/stick-autoboot.md#what-the-script-runner-test-showed-2026-11-15)): the prompt came once Akai's app was up, and the script ran as root through `systemd-run` (a transient `autorun-sh.service`), from a FAT32 stick mounted `noexec` (a shell script is read by `sh`, so `noexec` doesn't stop it). It was rejected as the *boot* trigger because it needs a tap on every boot after Akai's app has drawn. For a **one-time setup** that tap is exactly what we want.

### The setup script

The creator adds an exFAT partition, `MPFREE-SETUP`, after the stick's four partitions (partition 1 must stay `mpfree-test-root`, as the hook requires). It is about 8 GiB, so the backup fits (exFAT, because FAT32 can't hold a file over 4 GiB; Linux, macOS, Windows and Akai's system all read it). It holds:

- `autorun.sh`: a thin wrapper that runs the steps below with `sh` and writes everything it prints to `MPFREE-SETUP/setup-log.txt`;
- the backup step's script, read-only on the MPC ([below](#the-emmc-backup-and-the-mpcs-own-drum-kits));
- `hook-install.sh`, byte for byte as on main, so the files it writes and its undo stay exactly the ones already tested;
- `mode.txt`: `install` or `remove`, set by the app (so the same stick can uninstall; see below).

The wrapper runs, in this order, and stops at the first failure, writing why:

1. **Checks** (read-only): the board is an MPC One (`/proc/device-tree/model` and `compatible`), the kernel release equals the stick's `/boot/mpfree-softreboot/release`, no firmware updater is running, and the setup partition has room for the backup.
2. **The eMMC backup** (read-only on the MPC; writes only the stick). A setup whose backup fails or doesn't verify changes nothing on the MPC.
3. **The hook install**: `hook-install.sh` writes its three files on `/data` and runs the hook's own read-only check of the stick, ending in a `RESULT:` line. This is the only change to the MPC.
4. **The restart**: once no firmware updater is running and nothing is left at `/run/nextroot` (where a plain reboot would turn into a soft-reboot), `systemctl reboot`. The MPC restarting by itself is the musician's sign that the setup has finished; with the stick in, MPFree starts.

The script never writes the root filesystem, the boot partitions or the U-Boot environment.

**Showing the result.** Akai's app owns the screen while the script runs, and whether it shows anything while a script runs or when it ends is not known, so the musician is told before tapping Yes that it takes a while and that the MPC restarts by itself when done. The result is shown in two places that don't depend on Akai: MPFree's first screen after the restart reads `setup-log.txt` from the stick and says "MPFree is set up" or what went wrong, and the app shows the same log when the stick is next plugged into the computer. If the setup stops early, the MPC doesn't restart and stays in Akai's app; the log says why.

**Uninstall.** Taking the stick out is always enough to get Akai's MPC back. To remove the hook's files too, the app's **Remove MPFree from my MPC** button sets `mode.txt` to `remove`; the next run of the setup script (developer mode on, tap Yes) runs `hook-remove.sh`, which deletes only files carrying the hook's marker line.

### The eMMC backup and the MPC's own drum kits

**The backup.** The setup script does on the MPC what `host_emmc_backup.py` does over SSH today, with the stick as the destination instead of the SSH stream:

- `dd` reads `/dev/mmcblk0` (the 7.28 GiB user area), `mmcblk0boot0` and `mmcblk0boot1` (4 MiB each) into `MPFREE-SETUP/backup-<date>/`, with `SHA256SUMS` and a `MANIFEST.txt`. On the MPC devices are only ever `dd`'s input; nothing on the eMMC is mounted, written, synced or restarted.
- The same checks as today: before copying, the disk layout and eMMC model match what MPFree expects; the MPC hashes p1 to p6, boot0, boot1 and the GPT head itself, and the copies on the stick, read back, must match; the write and discard counters of p1 to p6, boot0 and boot1 must not move during the copy; the MPC must not restart in between (`boot_id`).
- p7 (content) and p8 (data) are mounted read-write by Akai's system, so, as today, they are captured like a power-loss snapshot. Akai's app is sitting at its prompt during the copy, so little is written.
- Time: today's backup over Ethernet takes about 4 minutes. To a stick on the MPC One's USB 2.0 port it is roughly 5 to 25 minutes, depending on the stick's write speed. The app can test the stick's write speed while creating it and tell the musician how long to expect.
- **The backup belongs on the computer**, not only on the stick. When the stick is next plugged in, the app copies it to a folder the musician can see (Documents/MPFree backups), checks every hash, and only then deletes it from the stick to free the space. A musician who never returns to the computer still has it on the stick.
- What it is for, said plainly in the app: a full copy of the MPC as it was, including the musician's own projects and settings on p8. It is the starting point for any future recovery, but restoring the boot parts of the eMMC is not possible yet: MaskROM recovery is unproven ([recovery tiers](../research/recovery-and-os-strategy.md)). The backup holds the musician's data and the unit's identifiers, so it stays on their own computer and is never uploaded.

**The MPC's own drum kits.** `offline-factory-kit.py` already makes kits from a backup: it finds the content partition (p7) in `mmcblk0.img`, reads it read-only, picks drum programs (`.xpm` files of type Drum) and converts their samples to the pad player's kit format. The plan ports that to Rust as one library used in two places:

- **MPFree, at its first start on the MPC**, finds the backup on the stick's setup partition, reads the factory drum kits out of it and saves them on partition 4 (MPFree's own data partition), showing progress on the screen. The eMMC is switched off by then; MPFree reads only the copy on the stick. So the musician has the MPC's own sounds from the very first start, with no step of their own.
- **The app on the computer** runs the same library on the copied backup if the MPC didn't (for example when the stick was rewritten), and puts the kits on the stick's partition 4.

The kits stay on the musician's stick and computer; nothing is uploaded or shared. MPFree's own kits (rendered from the Drums synth, plus CC0 samples) are always there too, so the pad player works even before or without the import.

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

1. **Bundle e2fsprogs** (`debugfs`, `mke2fs`, `e2fsck`) built for each system and run them as separate programs, keeping the builder's tested command list. They are GPL-2; shipping them as separate programs next to an MIT app is fine with their source offered. **Chosen first** (owner, 2026-11-15).
2. Write the narrow set of ext4 edits the builder needs (add files and links, replace files, change modes) in Rust. No outside programs, but a filesystem writer is a lot to get right.
3. Make partition 1 squashfs or erofs, which are simple to write from code. That changes how the hook mounts the stick (`ro,noload` is an ext4 option) and needs Akai's kernel to support it: a hook change, so the owner's call.

Whichever is used, the Python builder stays the reference: a test builds the same inputs both ways and checks that partition 1 holds the same files, links, modes and owners.

**Writing the stick** follows the device rules in the [installer design](usb-installer.md#product-goal) and `write-test-stick.py`: only whole USB disks the system calls removable, within a size range, nothing mounted from them, shown by name and size, a clear confirm, an exclusive open, a read-back compare, and partition 4 kept. Per system:

| System | How |
|---|---|
| Windows | The writer runs as administrator (UAC prompt); locks and dismounts the stick's volumes, writes `\\.\PhysicalDriveN` |
| macOS | `diskutil unmountDisk`, then the raw device `/dev/rdiskN` opened through the system's authorization prompt |
| Linux | `udisks2` (the desktop's password prompt), or `pkexec` as a fallback |

Raspberry Pi Imager (Apache-2.0) has solved these on all three systems and is the reference to read.

### What ships in the app

- MPFree's own programs for the stick (pad player, startup screen, screen log), built for ARM by CI and pinned by hash. Bundling them means one app version is one tested set and works offline.
- A table of supported Akai update files by SHA-256 and kernel release, deny by default. A hash is not Akai's content.
- **MPFree's own drum kits**, with a provenance list. The pad player can't start without `kits.bin` today, and the kits on the reference stick are Akai's factory sounds, copied out of the owner's eMMC backup by `offline-factory-kit.py`. A musician has no such backup, and Akai's sounds can't ship. The choices (the owner chose **both**, 2026-11-15):
  - **Rendered from MPFree's Drums synth.** CI plays each pad of a few Drums kits once and writes the hits into `kits.bin` (the format in `rust/mpfree-app/src/kit.rs` is plain 44.1 kHz audio, so nothing in the pad player changes). Entirely MPFree's own, nothing to license, but synthetic in character (808/909-style).
  - **CC0 samples.** Public-domain recordings of real drums. More realistic, but each file needs its source and licence recorded, and someone has to choose them.
  - **Both** (chosen): synthetic kits as the default, plus a few CC0 kits.
  - On top of these, **the MPC's own factory kits**, made automatically from the musician's eMMC backup ([above](#the-emmc-backup-and-the-mpcs-own-drum-kits)). They never ship with the app.

### Developer SSH

The app's **Preferences > Developer options** has one checkbox, **Developer SSH on the stick** (off by default). When ticked, the app asks for the user's SSH public key file, or makes a key pair for them and saves it where they choose, and the stick gets today's key-only root SSH on port 2222 with a host key made for that stick. The app then shows the exact `ssh` line to connect. Without the checkbox the stick has no SSH server at all. The user manual's page on it gives step-by-step instructions for Linux, macOS and Windows (all three ship `ssh`), including how to connect the MPC by cable.

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
| A board check in the hook and the setup script: MPC One only | **Hook change: needs the owner's go** | Akai's Gen1 image covers eight boards, and the switch-off service names the MPC One's controller addresses. Today the hook checks only the kernel release |
| Does the hook survive switching developer mode off? | Read-only check on the device (`--hook-report` after the restart) | The flow depends on it |
| Does the script runner offer `autorun.sh` from an exFAT partition that is partition 5 of a GPT stick with ext4 partitions, and can the script write 8 GiB to it? | Test with a harmless script | The one-stick design and the backup depend on it |
| Can MPFree read the setup partition after the soft-reboot? LoadPin then refuses every module load, so the exFAT driver must be built into Akai's kernel or already loaded | Read-only: Akai's kernel config, from the update file on the workstation (`offline-kernel-config.py`) | The first-start kit import and the setup result screen depend on it. If not: the hook preloads the module (`preload-modules.txt`), or the app on the computer does the import |
| Does Akai's app stay usable, or hang, while a script runs for 5 to 25 minutes? Does it time the script out? | Test with a harmless long script | The backup runs inside the script runner's session |
| A backup to the stick from the MPC: the same checks as `host_emmc_backup.py`, rehearsed on a fake MPC like the hook's tests | Workstation tests, then one read-only run on the reference MPC | The backup is the first thing a musician's MPC runs |
| What Akai's system does with the stick's partitions while it sits in during setup (mounts, `no-reserved-blocks@` tune2fs, writes to partition 4) | Read-only, from a stock-boot journal | During setup the stick stays in under Akai's app for a minute or more |
| Does `/data/system/etc/overlay` survive an Akai update? | Watch on the next Akai update | Either way the hook refuses the stick (release check) and Akai's MPC starts; the app then needs a new supported image |
| A soft-reboot-only build mode (no `live.dtb`), SSH only when asked for, MPFree's own kits, the setup partition | Builder changes, workstation only | The creator's build |
| The factory-kit import in Rust (from `offline-factory-kit.py`), with the Python tool as the reference: the same backup must give the same kits | Workstation tests | Used by MPFree on the MPC and by the app |

## Plan in stages

1. **Prove the flow by hand on the reference MPC, no app yet.** Builder changes (soft-reboot-only mode, SSH only when asked for, own kits, setup partition and script with the backup step), the board check (on the owner's go), then: build, write, developer mode on, tap Yes, wait for the restart, check the backup against the owner's last SSH backup, developer mode off.
2. **The factory-kit import in Rust**, in MPFree's first start and as a library for the app.
3. **The Rust core and command line**, checked against the Python builder.
4. **The GUI and the writers**, with the backup copy to the computer: Linux first, then Windows and macOS.
5. **Signing, the manual pages, and a small beta** with a few musicians.

## Still open

1. Signing: launch free as in [Releases](#releases), or pay for an Apple Developer ID?
2. The board check in the hook (a hook change) needs the owner's go before stage 1.
