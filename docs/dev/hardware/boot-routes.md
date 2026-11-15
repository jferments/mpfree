# Boot routes on MPC OS 3.10.1: soft-reboot, kexec_file, our own kernel

**Status:** 2026-11-15. The soft-reboot works and is the default. The kexec-file route is opt-in; its first round on the MPC restarted on Akai's kernel with LoadPin off, but Ethernet did not come up. The chain's first step (Akai's kernel restarted again with the stick's own device tree, `--chain`) is built and opt-in, not yet tried; our own kernel comes after it. The U-Boot route is not built. **Device:** the reference MPC One, Akai MPC OS 3.10.1 installed (kernel `6.18.44-az01-2026-08-10-rt6`).

This page compares the ways MPFree can take over the MPC after power-on. It explains the kernel policy that decides between them (LoadPin), and what each way means for running our own kernel and for external hardware that needs its own drivers. How the soft-reboot works in detail is in [How MPFree boots from the stick](soft-reboot-boot.md). The history and the owner's runbooks are in [the stick auto-boot record](stick-autoboot.md).

## The short version

- Every route starts the same way. Akai's U-Boot boots Akai's kernel, a [hook](soft-reboot-boot.md#the-hook-on-akais-data-partition) on Akai's data partition sees the stick about 6 s in, checks it and hands over. The routes differ only in what the handover does.
- **Soft-reboot** (works): keeps Akai's kernel and replaces everything above it. It is the only route proven on the MPC. Its limit is that no module or firmware can be loaded after the switch.
- **kexec_file** (opt-in, first round passed without Ethernet): restarts Akai's own kernel with a command line we choose. It starts fresh, LoadPin included, and a module loaded from the stick. It is the only way found to put a word such as `loadpin.enforce=0` on a kernel command line without touching U-Boot.
- **Our own kernel** (first step built): a chain of kexec_file into Akai's kernel with LoadPin relaxed, then an ordinary kexec into MPFree's kernel and device tree. Akai's kernel honours `loadpin.enforce=0`, so LoadPin allows it. The built first step (`--chain`) restarts into the stick's own device tree on Akai's kernel; our own kernel is a later step.
- **U-Boot** (blocked): booting our kernel straight from the bootloader. It is the cleanest route, but it writes boot-critical storage, which the [safety rule](../../agents/README.md) keeps behind proven MaskROM recovery.

## LoadPin: the policy that decides

LoadPin is a Linux security module. It allows the kernel to load files (kernel modules, firmware, kexec kernels and their initramfs) only from one trusted filesystem. On the installed 3.10.1:
- it is built in and enforcing (`CONFIG_SECURITY_LOADPIN_ENFORCE=y`, `/proc/sys/kernel/loadpin/enforce` is 1). It is pinned to Akai's dm-verity root, and also trusts the verity digests that `az0x-loadpin-trustpoints` hands it (`CONFIG_SECURITY_LOADPIN_VERITY=y`);
- it refuses the old `kexec_load` system call outright, because that call passes the kernel as a memory buffer with no file to check (`LoadPin: kexec-image old-api-denied`). That refusal closed the [first hook](stick-autoboot.md#the-hook-step-3-the-kexec-version-replaced);
- it accepts `kexec_file_load` of a kernel file on the trusted root, such as Akai's own `/boot/zImage-<release>`, and refuses one on the stick;
- its `enforce` setting cannot be lowered at runtime while the pinned root is read-only. It can be set only on the kernel command line (`loadpin.enforce=0`, or `loadpin.exclude=kexec-image,kernel-module,firmware` for chosen kinds). That command line comes from U-Boot, which is off limits;
- when the pinned filesystem is unmounted, it refuses **every** later load (`umount pinned fs: refusing further loads`). That is what happens at a soft-reboot.

When a kernel boots fresh with the stick as its root, LoadPin pins the first filesystem it loads a file from, which is the stick's. This was **observed** in the 3.10.1 stick test: the same 3.10.1 kernel, started fresh by kexec from 3.6.0, pinned the stick's partition 1 ([record](firmware-3.10.1-stick-test.md)). This is the main reason kexec_file may matter.

## The routes in detail

### 1. Soft-reboot (default, works)

systemd stops Akai's programs and starts the stick's systemd as PID 1 on the kernel that is already running ([details](soft-reboot-boot.md)).
- **Kernel, device tree, command line:** Akai's, unchanged. The stick's userspace is a modified copy of Akai's root filesystem for the same release, so it matches the kernel.
- **LoadPin:** stays pinned to Akai's root, which is detached at the switch, so from then on nothing loads. Modules Akai's boot loaded in its first 6 s stay. The hook can load more from Akai's root before the switch (`--preload-module`), but only modules that Akai's image contains, and only ones named when the stick is built.
- **The eMMC and SD card:** visible, because the device tree is Akai's. The stick's first service unbinds both controllers before udev or any mount, and powers the MPC off if it can't.
- **Inherited state:** the udev database in `/run` survives, so the stick's systemd takes over Akai's device units. Any Akai unit that can touch a disk must be masked on the stick; one read the SD card before the mask.
- **Speed:** about 16 s from power-on to the pad player.
- **After an Akai update:** the hook refuses the stick until it is rebuilt for the new release.

### 2. kexec_file of Akai's own kernel (opt-in, first round passed)

A stick built with `offline-kexec-stick.py --kexec-file` asks the hook to load Akai's `/boot/zImage-<release>` with the file-based kexec and the stick's command line, then start `kexec.target`. If that is not possible it soft-reboots instead ([how](soft-reboot-boot.md#the-opt-in-kexec-file-route), [runbook](stick-autoboot.md#the-kexec-file-route-step-6-opt-in)).
- **Kernel:** Akai's, started again. The hook first checks that its hash is the one the stick was built with.
- **Device tree:** Akai's, reused. kexec-tools refuses a device tree in file mode on 32-bit ARM (`DTB file cannot be specified in file mode`), so the eMMC and SD card stay visible and the same switch-off service does the same job.
- **Command line:** ours. `root=` points at the stick, and the hook allows only a short list of words. `--loadpin-off` adds `loadpin.enforce=0`.
- **LoadPin:** starts afresh. Without `loadpin.enforce=0` it is expected to pin the stick's root, as in the stick test, so the stick's modules and firmware load. With it, nothing is pinned and every load is allowed. Akai may have changed LoadPin in their build; the boot log line `LoadPin: ready to pin (currently enforcing)` or `(currently not enforcing)` shows which.
- **Inherited state:** none. `/run` and the udev database start empty, and no driver state from Akai's app carries over.
- **Cost and risks:** a second kernel start (a few seconds). Ethernet does not find its PHY on Akai's device tree. If the new kernel panics, the MPC restarts and tries again until the stick is pulled.
- **Shown on 2026-11-15** ([first round, zip 192320Z](stick-autoboot.md#first-round-2026-11-15-zip-192320z-the-jump-worked-loadpin-off-no-ethernet)): the install check loads and unloads Akai's kernel with the stick's command line. Both power-ons restarted on it and reached the pad player with no tap. `LoadPin: ready to pin (currently not enforcing)`, so Akai's kernel takes `loadpin.enforce=0`. LoadPin pinned the stick's root, and a module (`cfg80211`) loaded from it. The eMMC and SD card were only probed before the switch-off service turned them off, at 4.0 s and 7.3 s. The saved setting came back. Ethernet stayed down: `MDIO device at address 1 is missing`.

### 3. Our own kernel, by a chain (first step built: our device tree)

Building on route 2:
1. The hook kexec_file-loads Akai's kernel with `loadpin.enforce=0` (or `loadpin.exclude=kexec-image`) and **without** `sysctl.kernel.kexec_load_disabled=1`, and a small first stage on the stick as root.
2. That stage loads MPFree's own kernel and device tree with the ordinary `kexec_load`, which LoadPin then no longer refuses, and jumps.

**Built, opt-in, untried:** `offline-kexec-stick.py --kexec-file --loadpin-off --chain`. The first step uses the stick's own device tree (`/boot/mpfree-kexec/kexec.dtb`: the Ethernet PHY's ID, the eMMC and SD controllers disabled) with Akai's kernel, a byte-for-byte copy on the stick. That fixes Ethernet and moves the eMMC and SD switch-off into the tree. Our own kernel is the later step: a different `zImage` in the same place. How it works, its risks and its runbook are in [step 7](stick-autoboot.md#the-chain-to-the-sticks-own-device-tree-step-7-opt-in).

What it gives:
- any kernel configuration, any driver, and a newer kernel;
- **our own device tree**, so the eMMC and SD controllers are disabled in hardware description, as in the 3.6.0 stick test, instead of switched off by a service.

What it costs:
- two extra kernel starts and two kexec handovers (the second is the kind the stick test proved from 3.6.0's kernel);
- maintaining a kernel for the MPC's hardware: the RK3288 is well supported upstream, but every MPC-specific part (audio codec, display panel, touchscreen, the controller's quirks) must work and stay working;
- the [recovery tiers](../research/recovery-and-os-strategy.md#recovery-tiers) list "another kernel" under Tier 3. Booting one in RAM leaves the eMMC untouched, but the owner decides whether that counts.

It is possible only if Akai's kernel takes `loadpin.enforce=0` from the command line. Upstream Linux does, and so does Akai's: the first `--loadpin-off` run (zip 192320Z) logged `LoadPin: ready to pin (currently not enforcing)`.

### 4. U-Boot (blocked by the safety rule)

Changing U-Boot's environment or boot script would boot MPFree's kernel directly, with any command line and device tree, and no Akai kernel at all. It is the fastest and cleanest route, and the one a finished MPFree install would want. It writes the U-Boot environment or the boot area, the part of the eMMC that only MaskROM can recover. It stays blocked until that recovery is proven on this unit and the owner says go ([bootloader](bootloader.md), [recovery tiers](../research/recovery-and-os-strategy.md)).

### Also considered

Running MPFree under Akai's own systemd (chroot or namespace) keeps Akai's kernel and LoadPin as they are and is the fallback if soft-reboot ever fails. Akai's units keep running and can write `/data`, so the soft-reboot was preferred.

## Side by side

| | Soft-reboot | kexec_file | Chain to our kernel | U-Boot |
|---|---|---|---|---|
| Status | **Works, default** | Opt-in; restarts, no Ethernet | Our device tree: built, untried; our kernel: not built | Blocked |
| Kernel | Akai's, running | Akai's, restarted | **Ours** | **Ours** |
| Device tree | Akai's | Akai's | **Ours** | **Ours** |
| Command line | Akai's | **Ours** (checked list) | Ours | Ours |
| eMMC and SD card | Switched off by a service | Switched off by a service | Disabled in the device tree | Disabled in the device tree |
| Modules after handover | None (preload from Akai's image only) | From the stick (seen: `cfg80211`) | Any | Any |
| Firmware after handover | None | Expected: from the stick | Any | Any |
| Akai state carried over | udev database, device units | None | None | None |
| Extra boot time | none | one kernel start | two kernel starts | none (faster) |
| Writes on the MPC | hook files on `/data` | hook files on `/data` | hook files on `/data` | **boot area or U-Boot environment** |
| Tied to Akai's release | stick rebuilt per release | stick rebuilt per release; kernel hash must match | first stage tied; our kernel not | no |
| Undo | pull the stick | pull the stick | pull the stick | needs recovery |

## External hardware that needs its own drivers

What a USB device needs from the kernel decides which routes can use it.

| Kind of device | What it needs | Soft-reboot | kexec_file | Our own kernel |
|---|---|---|---|---|
| Class-compliant USB audio and MIDI (most audio interfaces and keyboards) | `snd-usb-audio`, which the MPC's own controller already uses | Works | Works | Works |
| Keyboards, game controllers, USB storage | Standard drivers that are normally loaded at boot | Very likely works | Works | Works |
| A device whose driver is in Akai's image but not loaded in the first 6 s | Loading a module | Only if named at build time (`--preload-module`), loaded at every boot whether the device is there or not | Loads when plugged in (expected) | Works |
| A device whose driver is not in Akai's image (a vendor-specific audio interface, a USB-serial chip, a USB network adapter) | A module built for Akai's exact kernel | Impossible | Possible if built against Akai's kernel source and configuration (`/proc/config.gz`); whether Akai requires signed modules is unknown | Works: build it in |
| A device that needs firmware loaded (Wi-Fi, Bluetooth, some interfaces with DSPs) | A firmware load | Only if the device was there and loaded it before the switch | Expected: from the stick's `/lib/firmware` | Works |
| Newer driver fixes and quirks (for example a new USB audio quirk) | A newer kernel | No | No | Yes |

So the soft-reboot covers the class-compliant devices most musicians use. kexec_file widens that to anything Akai's kernel can drive, as long as it is not stopped by LoadPin changes or module signing that Akai may have made. Our own kernel is needed for drivers Akai's kernel cannot have, and for owning the device tree.

## What the next runs answer

| Question | How it is answered | Answer (2026-11-15, zip 192320Z) |
|---|---|---|
| Does Akai's kernel accept kexec_file with our command line? | The install check of a `--kexec-file` stick (load and unload, RAM only) | Yes |
| Does the jump come up on Akai's device tree? | The first `--kexec-file` power-on | Yes, on both power-ons |
| Does `loadpin.enforce=0` take? | That power-on's `LoadPin: ready to pin (currently ...)` line in the stick's `dmesg.txt` | Yes: `currently not enforcing` |
| Do modules and firmware load from the stick? | The same `dmesg.txt`: no LoadPin refusals | A module did (`cfg80211`), with no refusal; no firmware was asked for |
| Does Ethernet find its PHY? | The same log, and whether SSH on port 2222 answers | No: `MDIO device at address 1 is missing` |
| Does Akai's kernel require signed modules? | A read-only grep of `/proc/config.gz` for `CONFIG_MODULE_SIG_FORCE` | Not run yet |

`loadpin.enforce=0` takes, so LoadPin no longer blocks the chain to our own kernel. The owner chose it as the next step, starting with the stick's own device tree on Akai's kernel: that gives the Ethernet PHY its ID and disables the eMMC and SD controllers in the tree.
