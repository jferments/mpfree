# The MPFree USB creator: a boot stick with no terminal

**Status:** proposal, 2026-11-15, waiting for the owner's review. Nothing here is built. It plans a desktop app for Linux, macOS and Windows that turns a musician's own copy of Akai's 3.10.1 update into an MPFree boot stick, and a one-time, terminal-free way to set up the MPC for it. The boot itself is the merged soft-reboot route in [How MPFree boots from the stick](../hardware/soft-reboot-boot.md); this page changes nothing about it.

## The goal

An average musician who wants to try MPFree should get from "my MPC runs 3.10.1 and I have Akai's update file" to "MPFree starts when the stick is in" without a terminal, SSH, keys, scripts or a network cable. The project never hosts or bundles any Akai file.

Out of scope: updating the MPC to 3.10.1. The musician does that first with Akai's own instructions and keeps the update file Akai gave them. The creator starts from there.

## What the musician does (recommended flow)

| Where | Step | Typing |
|---|---|---|
| Computer | Install and open **MPFree USB Creator**. Drop in Akai's 3.10.1 update file (the same one they updated the MPC with). The app checks it is exactly a supported file and says so in plain words. | none |
| Computer | Plug in a USB stick (8 GB or larger). The app lists only removable USB disks, with their name and size. Pick it, click **Create**, approve the system's password prompt. The app builds the stick, writes it and reads it back. | none |
| MPC, once | Switch on developer mode (Menu > Preferences > Enhancements, then SHIFT + MENU > Developer). Plug the stick in. The MPC asks whether to run the stick's setup script: tap **Yes**. It sets the MPC up, which takes a few seconds. | none |
| MPC, once | Switch developer mode off again. The MPC restarts, and with the stick in, MPFree starts; its first screen says the setup worked. | none |
| From then on | Stick in at power-on: MPFree. Stick out: Akai's MPC, exactly as before. | none |

The one tap on the MPC is the owner's consent, which the [installer design](usb-installer.md) asks for anyway (`SHOW_DEVICE_CONSENT`).

## What today's process needs, and what the creator replaces

Today's owner sequence is blocks 4.1 to 4.5 of [the stick auto-boot record](../hardware/stick-autoboot.md#blocks-for-the-owner). Each step, checked against the code on main (d1fb37b):

| Step today | Needs SSH or developer mode? | Needs Akai's files? | In the creator |
|---|---|---|---|
| Unpack the update (`offline-update-extract.py`) | No | Yes: the user's update file | The app does it on the computer, from the user's file |
| Copy `/boot` from the image (`offline-rootfs-survey.sh`) | No | Yes: from the same file | The app does it |
| `collect-over-ssh.sh --kexec-preflight` for `live.dtb` | **Yes** | No | **Not needed.** `offline-kexec-stick.py` stops without it, but it only feeds `kexec.dtb`, which the soft-reboot route never uses. The soft-reboot check needs only the kernel release, and on an installed 3.10.1 that is the image's own. Needs a soft-reboot-only build mode |
| Drum kits (`offline-factory-kit.py`) | **Yes**: it reads the owner's eMMC backup, made over SSH | Yes: Akai's factory sounds | **Not possible** for a musician, and Akai's sounds can't ship. MPFree needs its own kits (below) |
| Your SSH key for port 2222 (`MPC_KEY`) | Only to use it | No | Off by default; a developer checkbox adds it |
| Build tools: `debugfs`, `mke2fs`, `e2fsck`, `sfdisk`, `ssh-keygen` | No | No | Linux tools today; the app must bring them or replace them on all three systems |
| `write-test-stick.py` (Linux, `lsblk`, sudo) | No | No | A writer per operating system, inside the app |
| `--devmode-trust` (host key, [Block Q1](../hardware/firmware-3.10.1-qualification.md)) and `--hook-install` over port 22 | **Yes** | No | **Replaced** by the setup script on the stick, run by Akai's script runner. No SSH at all, so no host key to trust |
| `--hook-report` over SSH | **Yes** | No | The setup script and the stick's log partition keep the notes; MPFree's first screen and the app show them |

So the stick build already depends only on the user's own update file, apart from the preflight (not needed) and the kits (replaced). The hook install is the only step that truly needs the MPC, and it is the one the next section solves.

## Installing the hook without a terminal

**Yes, it can be done with no terminal: by the stick itself, through Akai's own script runner.** On 3.10.1, with developer mode on, Akai's app offers to run `autorun.sh` from a USB drive and runs it as root after one tap. This was tested on the reference MPC on 2026-11-15 ([record](../hardware/stick-autoboot.md#what-the-script-runner-test-showed-2026-11-15)): the prompt came once Akai's app was up, and the script ran as root through `systemd-run` (a transient `autorun-sh.service`), from a FAT32 stick mounted `noexec` (a shell script is read by `sh`, so `noexec` doesn't stop it). It was rejected as the *boot* trigger because it needs a tap on every boot after Akai's app has drawn. For a **one-time setup** that tap is exactly what we want.

### The setup script

The creator adds a small FAT32 partition, `MPFREE-SETUP`, after the stick's four partitions (partition 1 must stay `mpfree-test-root`, as the hook requires). It holds:

- `autorun.sh`: a thin wrapper that runs the steps below with `sh` and writes everything it prints to `MPFREE-SETUP/setup-log.txt`;
- `hook-install.sh`, byte for byte as on main, so the files it writes and its undo stay exactly the ones already tested;
- `mode.txt`: `install` or `remove`, set by the app (so the same stick can uninstall; see below).

Before calling `hook-install.sh`, the wrapper refuses, writing why, unless: the board is an MPC One (`/proc/device-tree/model` and `compatible`), the kernel release equals the stick's `/boot/mpfree-softreboot/release`, and no firmware updater is running. Then `hook-install.sh` writes its three files on `/data` and runs the hook's own read-only check of the stick, ending in a `RESULT:` line. The script never writes the root filesystem, the boot partitions or the U-Boot environment.

**Showing the result.** Akai's app owns the screen while the script runs, and whether it shows anything when a script ends is not known. So the result is shown in two places that don't depend on Akai: MPFree's first screen after the restart reads `setup-log.txt` from the stick and says "MPFree is set up" or what went wrong, and the app shows the same log when the stick is next plugged into the computer. If the MPC restarts into Akai's app instead, the log says why.

**Uninstall.** Taking the stick out is always enough to get Akai's MPC back. To remove the hook's files too, the app's **Remove MPFree from my MPC** button sets `mode.txt` to `remove`; the next run of the setup script (developer mode on, tap Yes) runs `hook-remove.sh`, which deletes only files carrying the hook's marker line.

### Options considered for the hook install

| Option | Verdict | Why |
|---|---|---|
| **A. Setup script on the stick, run once by Akai's script runner** | **Recommended** | No terminal, no network, no keys. One tap. Uses Akai's documented developer mode. Reuses `hook-install.sh` unchanged |
| B. The app connects to the MPC over the network (developer-mode SSH) | Developer path only | Needs a cable or Wi-Fi, finding the MPC's address, firewall prompts, and Akai's SSH, which takes root with no password until locked. Today's tools already do this for developers |
| C. The developer panel's on-screen terminal, typing one command | Rejected | Still a terminal |
| D. No hook: the script runner starts MPFree on every boot | Rejected by the owner | A tap on every boot, after Akai's app is up |
| E. A unit in Akai's root filesystem, or U-Boot booting USB | Forbidden | Root filesystem and U-Boot writes ([safety rules](../../agents/README.md)) |

### Developer mode afterwards

While developer mode is on, the MPC runs root SSH with no password on any network it is connected to ([compatibility page](../hardware/firmware-3.10.1-compatibility.md)). The flow therefore ends by switching it off. Akai's `disable.sh` deletes only `/data/root` and `/data/ssh` and restarts; the hook's files live in `/data/system/etc/overlay` and `/data/mpfree-hook`, so they should survive. **That has not been seen on the device yet** and is the first thing to check (below).

## The creator app

### Shape

| Option | Verdict | Why |
|---|---|---|
| **1. One app that builds and writes** | **Recommended** | One download, one window. It can keep the stick's saved settings (partition 4) when rewriting, as `write-test-stick.py` does |
| 2. A builder that makes `stick.img`, written with Raspberry Pi Imager or balenaEtcher | Fallback | Avoids writing our own disk writer, but it is two apps, and those writers erase partition 4 every time |
| 3. The Python tools packaged with a GUI wrapper (PyInstaller) | Prototype only | Fastest reuse, but it drags `debugfs`, `sfdisk` and Python packaging onto macOS and Windows |
| 4. A web page | Not possible | Browsers can't write raw USB disks |

### Technology

A Rust app (ADR-0001), split into a core crate (call it `mpfree-stick`: unpack the update, check its hash, copy and change the root filesystem, lay out the GPT image, verify) with a command-line front end for tests and CI, and a small wizard GUI. The project's own `mpfree-ui` draws to a framebuffer, so the desktop GUI would use a desktop toolkit; `egui`/`eframe` keeps it pure Rust, and Tauri is the alternative if a web-style UI is wanted.

**Editing ext4 on macOS and Windows** is the hardest part, because the builder changes a copy of Akai's ext4 root filesystem with `debugfs -w`. Options:

1. **Bundle e2fsprogs** (`debugfs`, `mke2fs`, `e2fsck`) built for each system and run them as separate programs, keeping the builder's tested command list. They are GPL-2; shipping them as separate programs next to an MIT app is fine with their source offered. **Recommended first.**
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
- **MPFree's own drum kits**, with a provenance list (made with MPFree's Drums synth or from CC0 samples). The pad player can't start without `kits.bin` today, and the current kits are Akai's sounds from the owner's backup.

### Releases

GitHub Releases with: a signed and notarized macOS app (needs an Apple Developer ID, US$99 a year; without it macOS refuses to open the app), a signed Windows installer (without a code-signing certificate SmartScreen warns on every download), and a Linux AppImage (a Flatpak later). A user manual page, "Installing MPFree", is written when the app exists.

## Akai firmware and legal limits

- **Nothing of Akai's is distributed.** The app reads the update file the musician already downloaded from Akai, on their own computer. It links to Akai's download page and never downloads or hosts the file itself, so the musician accepts Akai's terms directly. This matches the [factory content policy](../project/factory-content-policy.md).
- **The finished stick holds a changed copy of Akai's root filesystem.** It is the musician's personal copy, and the app says so: don't share stick images. The project never publishes a prebuilt image.
- **No factory sounds** on a creator-made stick. A later, optional importer of the owner's own content stays local and needs the legal review the policy already asks for.
- **Nothing is circumvented.** The route uses Akai's documented developer mode and script runner and Akai's own running kernel. It doesn't touch the eMMC boot partitions, the verified root filesystem, LoadPin, or any licensing or activation.
- **Needs real legal review before a public release** (not decided here): what Akai's terms say about changing a copy of the firmware for personal use and about studying its format; using Akai's product names (plain "for the Akai MPC One" wording, no logos, a "not affiliated with Akai or inMusic" notice); and the GPL source offer for anything we bundle.

## Safety work before anyone else runs it

| Item | Kind | Why |
|---|---|---|
| A board check in the hook and the setup script: MPC One only | **Hook change: needs the owner's go** | Akai's Gen1 image covers eight boards, and the switch-off service names the MPC One's controller addresses. Today the hook checks only the kernel release |
| Does the hook survive switching developer mode off? | Read-only check on the device (`--hook-report` after the restart) | The flow depends on it |
| Does the script runner offer `autorun.sh` from a FAT32 partition that is partition 5 of a GPT stick with ext4 partitions? | Test with a harmless script | The one-stick design depends on it. If not: a second small setup stick |
| What Akai's system does with the stick's partitions while it sits in during setup (mounts, `no-reserved-blocks@` tune2fs, writes to partition 4) | Read-only, from a stock-boot journal | During setup the stick stays in under Akai's app for a minute or more |
| Does `/data/system/etc/overlay` survive an Akai update? | Watch on the next Akai update | Either way the hook refuses the stick (release check) and Akai's MPC starts; the app then needs a new supported image |
| A soft-reboot-only build mode (no `live.dtb`), no SSH by default, MPFree's own kits, the setup partition | Builder changes, workstation only | The creator's build |

## Plan in stages

1. **Prove the flow by hand on the reference MPC, no app yet.** Builder changes (soft-reboot-only mode, no SSH by default, own kits, setup partition and script), the board check (on the owner's go), then: build, write, developer mode on, tap Yes, developer mode off, power on.
2. **The Rust core and command line**, checked against the Python builder.
3. **The GUI and the writers**: Linux first, then Windows and macOS.
4. **Signing, notarization, the manual page, and a small beta** with a few musicians.

## Open questions for the owner

1. Is switching developer mode on once (and tapping Yes) acceptable as the one MPC-side step?
2. One stick with a setup partition (recommended), or a separate small setup stick?
3. Kits for creator-made sticks: made with MPFree's Drums synth, CC0 samples, or both?
4. GUI toolkit: `egui` (pure Rust, recommended) or Tauri? Is bundling e2fsprogs acceptable?
5. Signing costs: an Apple Developer ID and a Windows code-signing certificate?
6. MPC One only for the first release?
7. Who does the legal review of Akai's terms, and does it gate the first public release?
8. Developer SSH on the stick: off by default with a checkbox, or left out of creator builds?
