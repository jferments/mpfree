# Starting MPFree from the stick at power-on

**As of 2026-11-15 (soft-reboot hook)** · **Device:** the reference MPC One, Akai MPC OS 3.10.1 installed on the eMMC (kernel `6.18.44-az01-2026-08-10-rt6`), developer mode on.

## The goal

The owner's goal for the first release (2026-11-14, sharpened 2026-11-15): with the MPFree stick in, switching the MPC on starts MPFree with no computer and no tap. The MPC logo may show for a moment, but Akai's app must not load first; MPFree's startup screen is the first thing after the logo. With the stick out, the MPC starts the factory system as before.

The jump was first the kexec that [the stick test](firmware-3.10.1-stick-test.md) uses: Akai's Linux loads the kernel from the stick with `kexec --load` and `systemctl kexec` starts it, with a device tree that hides the eMMC and SD controllers. On the installed 3.10.1, LoadPin refuses that load (below), so the hook now does a **systemd soft-reboot** into the stick's system instead, on Akai's running kernel ([the soft-reboot hook](#the-soft-reboot-hook-step-4)). Either way, the trigger that starts the jump at power-on, early enough, is the same.

## What can start it, and what is ruled out

| Route | Before Akai's app? | No tap? | Writes | Status |
|---|---|---|---|---|
| Akai's script runner (`autorun.sh` on a drive) | No: it asks once the app is up | No | none | Works (below), but the owner rejected the tap |
| A systemd unit in Akai's writable `/etc` layer (on `/data`) | — | — | `/data` | Ruled out: systemd reads its unit folders before `etc.mount` puts that layer over `/etc`; tried on 3.6.0, the unit did not start at boot ([MOD-001's notes](archive/2026-11/device-modifications.md)) |
| A systemd unit in the root filesystem (like MOD-001) | Yes | Yes | **root filesystem** | Blocked by the safety rule; an Akai update would also remove it |
| U-Boot looking at USB first | Yes, before Linux | Yes | **U-Boot environment** | Blocked by the safety rule until recovery is proven; whether Akai's U-Boot can read USB storage at all is unknown ([bootloader](bootloader.md)) |
| A **udev rule** in Akai's writable `/etc` layer | Ran at 6.42 s, just before the app was started | Yes | `/data` | Loads during boot (canary, first power-on); the margin is thin |
| A **modprobe `install` line** in Akai's writable `/etc` layer, for the Wi-Fi module `cfg80211` | Yes: ran at 5.96 s, its service at 6.09 s, the app was started at 6.36 s | Yes | `/data` | **Works** (second canary, power-on); the route for the hook |

### What the script runner test showed (2026-11-15)

A spare FAT32 stick with the harmless `autorun.sh` from [`write-autorun-drive.py`](../../../tools/mpc-one/write-autorun-drive.py), in the MPC at power-on: the MPC asked only once Akai's app was fully up. `az01-launch-MPC` logged `About to run unprotected script: /media/USB321FD/autorun.sh`, then ran it through `systemd-run` as a transient `autorun-sh.service` in `system.slice` (not in the app's cgroup), as root (`uid=0`), 26 s after power-on, with `/usr/sbin` on its `PATH`. The FAT32 stick was mounted `noexec` at `/media/<label>`. kexec is allowed on the installed system: `kexec_load_disabled` is 0, nothing is loaded, `/usr/sbin/kexec` ships; `/sys/kernel/security/lockdown` does not exist.

Akai's `/etc` is two overlays: `etc.mount` (lower `/etc`, upper `/data/system/etc/overlay`) and developer mode's read-only one on top (lower `/usr/lib/az0x/user-access/etc-overlay:/etc`). The upper still holds `mpfree-sshd.service` and its `multi-user.target.wants` link from the November 9 attempt on 3.6.0. They have never run at boot, for the reason above, and are harmless.

## Why a udev rule might work

udev is the part of Linux that reacts when a device, such as a USB stick, appears. Unlike systemd's unit folders, udev re-reads its rule folders while it runs. In systemd 255 (3.10.1 has 255 or later), `udevd` checks the `.rules` files in `/etc/udev/rules.d` and the other rule folders each time it starts queued events, at most every 3 s, counted from its own start (`manager_reload()`, `event_queue_start()` and `manager->last_usec` in `src/udev/udev-manager.c`; `udev_rules_should_reload()` in `udev-rules.c`). The check comes before the queued events run, so the event that sets it off already gets the new rules. Once `etc.mount` has put the writable layer over `/etc`, a rule kept there is seen from the next check on.

The timing decides whether it helps. On the installed 3.10.1 (journal of 2026-11-15, seconds since power-on): `udevd` starts at about 3.05 s (its "Using default interface naming scheme" line), its start-up pass ("Coldplug All udev Devices") ends at 3.16 s, `/data` is mounted at 4.20 s, the writable `/etc` at 4.37 s, and Akai's app program is started at 6.54 s. In the stick test's boots a stick that was in at power-on appeared at about 2.8 s (`sda: sda1 sda2 sda3 sda4`). So the stick's own `add` event is handled before `/etc` holds the rule, and the rule will most likely miss it. What can work is the first device event after about 6 s (udev's first check after its start): with the rule loaded by then, a hook can use that event to look for the MPFree stick, which is there by then. The deadline is what the owner sees: the jump must come before Akai's app draws its screen; the app program itself is already running from 6.5 s. A rule must also not do the jump itself: udev runs its programs under a system-call filter without `kexec_load` or `mount`, so a real hook asks systemd to start a short service (`systemd-run --no-block`) that does the work, the way Akai's launcher starts `autorun.sh`.

## Why a modprobe line might work better

udev's 3 s limit makes the rule's timing depend on when other events happen: on the first power-on the first event after udev's check point (6.11 s) came at 6.42 s, only 0.3 s later. Had it come before 6.11 s, the rule would only have loaded with the next event, a backlight change at 17.0 s.

The module loader has no such limit. kmod's `modprobe` reads `/etc/modprobe.d` every time it runs, and the kernel runs it (`/proc/sys/kernel/modprobe`) when it needs a module. An `install cfg80211 <command>` line makes it run the command instead of loading `cfg80211`, so the command can log, start the jump service, and then load the module as before with `modprobe --ignore-install cfg80211`. `cfg80211` loaded at about 6.4 s on the first power-on of the installed system, and at 6.8 to 7.3 s in each of the three stick-test boots of 3.10.1. If udev's own module loader loads it, the line may only count once udev has re-read its setup, which is the same 3 s check; the canary records who loaded the module.

Two rules keep it harmless. The line names the kernel's module loader by its full path, so that a shell which prefers its BusyBox applets still runs kmod's `modprobe` (BusyBox's ignores `install` lines and has no `--ignore-install`); the install checks that this path is kmod's. And nothing in the command waits for systemd: if systemd itself were the program waiting for `cfg80211`, a call to systemd would wait forever, so the command only reads `/proc` and `/sys` and starts the service in the background.

## The canary (steps 1 and 2)

Three modes of [`collect-over-ssh.sh`](../../../tools/mpc-one/collect-over-ssh.sh), on developer mode's port 22 only (`MPC_PORT=22`):

- `--hook-canary` (**state-changing, data partition only**; needs `MPFREE_ACCEPT_HOOK_CANARY=YES`) streams [`hook-canary-install.sh`](../../../tools/mpc-one/hook-canary-install.sh). It checks that the MPC runs 3.10.1 (kernel 6.18, az01) with `/etc` as the overlay above and that the kernel's module loader is kmod's `modprobe`, then writes three files and the folders they need, and records which folders it made:
  - `/data/mpfree-hook/canary.sh`
  - `/data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules`
  - `/data/system/etc/overlay/modprobe.d/99-mpfree-canary.conf`

  `canary.sh` only writes lines to the kernel log and to `/run/mpfree-canary.txt`, and a marker folder in `/run` (all RAM). Each line has the time since power-on and whether Akai's app has been started (read from `/sys` and `/proc`). It is run:
  - by the rule for every device event once udev has loaded it (logged in `/run` only); the first of a boot also goes to the kernel log and starts the short service;
  - by the rule for every USB disk partition that appears, with the partition's name; this also starts the service;
  - by the modprobe line when `cfg80211` loads, with the chain of programs that asked for it; this starts the service in the background, then the line loads `cfg80211` as before;
  - by systemd, as that short service (`systemd-run --no-block`, the path a real hook would use to start the jump), which logs one more line with the partition labels it can see.

  Nothing else is read, written, mounted, loaded or started. A rule udev cannot parse is skipped with a log message, and a modprobe line kmod cannot parse is skipped with a warning; neither can stop the MPC from booting. The rule passes `udevadm verify` (systemd 255) and kmod 31 reads the modprobe line as written (`modprobe --showconfig`). If `canary.sh` were missing, the modprobe line still loads `cfg80211`, since its parts are separated by `;`.
  If the write is cut off: the script is written first and the rule and modprobe line after it, each under a temporary name that udev and kmod ignore and then renamed, so neither points at a missing script. Running it again finishes the job, or reports that the files are already in place. It replaces the first version of the canary (two files, same marker line).
  It writes the upper folder directly, since the `/etc` that programs see is developer mode's read-only overlay. The overlay already mounted does not show the new files before the next power-on (seen on 2026-11-15: the rule was not in `/etc/udev/rules.d`, and a stick plugged in after the install was not noticed); nothing breaks either way.
- `--hook-canary-report` (read-only) streams [`read-only-hook-canary.sh`](../../../tools/mpc-one/read-only-hook-canary.sh): whether the canary was installed before this boot started (if not, it says to switch the MPC off and on first), the canary's files and hashes, whether `/etc` shows them, the kernel's module loader, its lines in the kernel log and `/run`, and this boot's timeline from the journal (when `/data` and `/etc` were mounted, when the stick appeared, when `cfg80211` loaded, when the canary's services and Akai's app were started).
- `--hook-canary-remove` (**state-changing, data partition only**, the undo) streams [`hook-canary-remove.sh`](../../../tools/mpc-one/hook-canary-remove.sh). It deletes the three files only if they carry the canary's marker line (the modprobe line and the rule first), and the folders the install made only if they are empty. Running it again is harmless; it also removes the first version. By hand: `rm /data/system/etc/overlay/modprobe.d/99-mpfree-canary.conf /data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules; rm -r /data/mpfree-hook`.

Nothing here touches the root filesystem, the boot partitions, the U-Boot environment or the rest of `/etc`. Every run on port 22 checks the MPC's SSH host key strictly against `~/.ssh/known_hosts`, where `--devmode-trust` records it once (behind the wrapper's MAC check). Until 2026-11-15 the canary and hook runs kept it in `~/mpfree-work/mpc-devmode-known_hosts` instead.

**Passes when**, after a power-on with a stick in, the report shows a `service (...) ran` line at a time before Akai's app draws its screen, which proves the path a jump would take, and the `modprobe loads cfg80211` line, which shows whether the module loader is the sturdier trigger.

**First run (2026-11-15, first version):** the install worked (both files written and read back, in the three folders it made), but the MPC was not switched off and on afterwards: it had been on for 646 s, and the stick went in 30 s after the install. As expected, nothing fired.

**First power-on (2026-11-15, first version, the MPFree test stick in):** the remove and a new install both worked on the device. After the power-on `/etc/udev/rules.d` showed the rule, and udev ran it from 6.42 s: `add` events for the module `cfg80211`, its class `ieee80211` and the device `regulatory`, 0.1 s before Akai's app was started (6.52 s). The next event was a backlight `change` at 17.02 s. The stick itself appeared at 2.79 s, before the rule was loaded, so the `udev saw` line and its service never ran: the path from a rule to a service at boot is still unproven. Timeline: `udevd` 3.11 s, start-up pass ended 3.25 s, `/data` 4.28 s, `/etc` 4.40 s, edisksd mounted the stick at 16.01 s. The second version's first report was again taken without a restart (12 s after the install); it showed who loads `cfg80211`: Akai's network service, connman, started at 6.14 s and the module loaded at 6.35 s.

**Second version, power-on (2026-11-15, the spare FAT stick in):** it passes. Both files showed in `/etc` after the power-on.
- The modprobe line ran at 5.96 s, called by the kernel itself (`sh < modprobe < kworker < kthreadd`), and the service it started ran at 6.09 s, with Akai's app not yet started. systemd logged `Started MPFree canary (modprobe)` at 6.07 s; Akai's app (`Started InMusic MPC Application`) followed at 6.36 s. connman had started at 5.78 s; `cfg80211` finished loading at 6.22 s, so the line held it up by about 0.25 s, which nothing noticed.
- udev's first event after loading the rule came at 6.28 s (the `add` events of `cfg80211`), and its service ran at 6.43 s, by when Akai's app was already running. Later events: the network queues' `remove` events at 11.3 to 11.4 s and the backlight `change` at 17.28 s.
- The service saw these partition labels: `content data env recoverysplash rootfs splash uboot uboot-spl` (the eMMC's; the spare stick has no GPT labels). The MPFree stick's labels (`mpfree-test-root` and so on) are set by udev at its start-up pass, before 3.3 s, so a hook can look for them at 6 s.

## The hook (step 3, the kexec version, replaced)

Built on what the second canary showed. This version jumps by kexec, which LoadPin refuses on the installed 3.10.1 (below); [step 4](#the-soft-reboot-hook-step-4) keeps its trigger, its files and its undo and replaces the jump. Three more modes of [`collect-over-ssh.sh`](../../../tools/mpc-one/collect-over-ssh.sh), on port 22 only:

- `--hook-install` (**state-changing, data partition only**, then a check that changes only RAM; needs `MPFREE_ACCEPT_HOOK=YES`) streams [`hook-install.sh`](../../../tools/mpc-one/hook-install.sh). It makes the canary's checks, plus that `/usr/sbin/kexec` is there and kexec is allowed, then writes three files in the canary's places, and deletes the canary's three files (only files carrying the canary's marker line; the canary's folder list is kept for the undo):
  - `/data/mpfree-hook/hook.sh`
  - `/data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules`
  - `/data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf`

  Then, with MPFree's stick in the MPC, it runs the hook's own check (`hook.sh check`): the steps the jump takes at power-on, with `kexec --load` followed by `kexec --unload` in place of the restart. Its `RESULT` line says whether the stick checks out. Run without the stick, it installs, says so and stops; run again, it writes nothing more and checks.
- `--hook-report` (read-only) streams [`read-only-hook-report.sh`](../../../tools/mpc-one/read-only-hook-report.sh): whether the hook was installed before this boot, its files and hashes, whether `/etc` shows them, whether the stick is in, what the hook noted at this power-on (`/run/mpfree-hook.txt` and the kernel log), this boot's timeline, and the boot before if the journal keeps it. After a jump that worked MPFree is running, so the report is for the power-ons that ended in Akai's system.
- `--hook-remove` (**state-changing, data partition only**, the undo) streams [`hook-remove.sh`](../../../tools/mpc-one/hook-remove.sh): it deletes the hook's files and any of the canary's, only those carrying their marker line, and the folders the installs made if they are empty. By hand: `rm /data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf /data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules; rm -r /data/mpfree-hook`. Taking the stick out is enough to get Akai's system at the next power-on, with the hook still installed.

What `hook.sh` does at power-on:

1. **Trigger.** The modprobe line runs it when `cfg80211` loads (about 6 s). The udev rule runs it for a device event, but only while `/run/mpfree-hook` does not exist and a partition named `mpfree-test-root` does. The first trigger in the first 15 s of a boot, with that partition present, makes `/run/mpfree-hook` (so it happens once per boot) and starts the jump service with `systemd-run --no-block`, in the background from the modprobe line, so that the module loads as before. No stick: the modprobe line notes it in `/run/mpfree-hook.txt` and nothing starts. Past 15 s (a stick plugged in later, a module loaded later): never.
2. **The jump service** (`mpfree-jump`, `Type=oneshot`, `TimeoutStartSec=45`) is ordered `Before=` `acvs.service`, `touch-fw-update.service`, `xmos-update.service` and `az01-usbsata-fixer.service`, so Akai's app and its firmware updaters wait while it runs. It:
   - finds the partition named `mpfree-test-root`, which must be partition 1 of a USB disk, and its partition UUID;
   - mounts it read-only (`ro,noload,nodev,nosuid,noexec`; or reads it where Akai's system already mounted it) and copies `/boot/mpfree-kexec` to `/run/mpfree-hook/kexec`: `zImage` (which must be a link to `../zImage-<release>` on the stick; the hook reads the link and copies its target, `/boot/zImage-<release>`, never following the link, because Akai's system mounts USB disks with `nosymfollow`), `kexec.dtb`, `cmdline.txt` and `SHA256SUMS`, each with a size limit, then unmounts it;
   - checks the copies against `SHA256SUMS`, and that the command line is one line of plain characters that starts the system on this very partition (`root=PARTUUID=` its UUID) with `ro`, `rootwait=60`, `panic=10` and `sysctl.kernel.kexec_load_disabled=1`;
   - waits, up to 20 s, while a firmware updater runs (one of those units activating, `dfu-util`, `touch-fw-upd`, `xmosdfu`, `midifirmup`, `az01-update`, `az01-usbsata-fixer` or Akai's `SME0/Firmware/update.sh`); if one is still running it gives up, since a restart must never cut a firmware write short;
   - loads the kernel (`/usr/sbin/kexec --load --kexec-syscall --dtb=… --command-line=…`), checks `kexec_loaded` is 1 and that no updater has started, and starts `kexec.target` (`systemctl --no-block --job-mode=replace-irreversibly start kexec.target`, what `systemctl kexec` does, without asking logind, which may not be running yet). systemd stops what has started (Akai's app, if it has), unmounts `/data` cleanly and starts the stick's kernel.

   On any problem it notes `STOP: <reason>` in the kernel log and `/run/mpfree-hook.txt`, unloads a loaded kernel, unmounts the stick, deletes its copies, and exits; Akai's app then starts as usual.

The stick carries the jump's files since this change: [`offline-kexec-stick.py`](../../../tools/mpc-one/offline-kexec-stick.py) adds `/boot/mpfree-kexec/` to partition 1 (the same `kexec.dtb` and `cmdline.txt` as in the build folder, a link to the image's own kernel, and their `SHA256SUMS`). It also builds for an MPC that runs the stick's own release, which is the installed 3.10.1 starting the 3.10.1 stick: then the running tree from `--kexec-preflight` is read against the same release's board file, and must be the installed system's (its eMMC controller enabled), not a tree read on a test build started by kexec. The device tree carries this unit's serial number and MAC address, as the build folder's always has: the stick stays private.

Tests: [`test_boot_hook.py`](../../../tools/mpc-one/tests/test_boot_hook.py) runs the wrapper, the installer, the undo and the report against a fake MPC, and the hook's triggers under BusyBox's shell; the jump and the check run under dash (BusyBox's shell runs its own `mount`, `sleep` and `pidof` before any stand-in) with stand-ins for `mount`, `kexec`, `systemctl` and `systemd-run`, through every way it can stop.

First install (2026-11-15): the three files were written and the canary deleted, but the check stopped with `the stick has no /boot/mpfree-kexec/zImage`. Akai's `edisksd` had mounted every partition of the stick, partition 1 included, read-write at `/media/<UUID>` with `nosuid,nodev,noexec,nosymfollow`, and the hook read the kernel through the link there. It now copies the link's target. A test makes the same `nosymfollow` mount in a private mount namespace (it skips where that is not allowed). At power-on the hook mounts the partition itself: `edisksd` mounted a USB stick at 8.8 s at the earliest on the installed 3.10.1, after the hook's copy at about 6 s.

Second install (2026-11-15, zip 075320Z): the check reached `kexec --load` and the kernel refused it: `kexec_load failed: Operation not permitted`, with `kexec_load_disabled` 0, as root. That is a kernel policy, not the tool. The successful stick-test kexecs on 2026-11-13 were launched from the MPC's then-installed **3.6.0** kernel; the owner installed **3.10.1** on the eMMC on 11-15.

The preflight (zip 082152Z) names the refuser exactly, and it is **LoadPin**, not IPE. The kernel log line is `LoadPin: kexec-image old-api-denied ... cmdline="/usr/sbin/kexec --load --kexec-syscall ..."`. LoadPin runs with `enforce` on (`/proc/sys/kernel/loadpin/enforce` is 1, `CONFIG_SECURITY_LOADPIN_ENFORCE=y`) and has pinned the dm-verity root (`az0x-loadpin-trustpoints` sets its trusted digests). Its `kernel_load_data` hook then refuses every raw `kexec_load` (the `--kexec-syscall` path, which passes a NULL file), and on a read-only pinned root the `enforce` sysctl cannot be lowered at runtime, only from the kernel command line. The earlier **IPE** theory does not hold on this unit: the preflight found `no /sys/kernel/security/ipe`, so although `ipe` is listed in `CONFIG_LSM` it is not active here. Lockdown is not the cause either (`CONFIG_SECURITY_LOCKDOWN_LSM is not set`). `CONFIG_KEXEC_FILE=y`, so `kexec_file_load` (`-s`) exists, but LoadPin accepts a file-based load only from the pinned or a trusted verity filesystem, which a plain stick is not. The hook's check caught the refusal and left Akai's system untouched; `--hook-report` confirms nothing started. (An earlier preflight read `/proc/sys/kernel/loadpin/enabled`, which does not exist; the file is `enforce`, now read.)

This closes the runtime-kexec route on 3.10.1: the boot hook depends on Akai's running kernel accepting `kexec_load` from the stick, and LoadPin on 3.10.1 does not. The only way to lift LoadPin is a kernel command-line word (`loadpin.enforce=0` or `loadpin.exclude=kexec-image`), which lives in U-Boot; the [safety rule](../../agents/README.md) puts a U-Boot-environment change behind proven MaskROM recovery and the owner's decision, so it is not done here.

What is still unknown, if the gate is ever cleared: whether `systemd-run` queues the service before Akai's app starts (the canary's service was queued about 0.3 s before it), how long the restart takes, and what the screen shows between the MPC logo and MPFree's startup screen.

## The soft-reboot hook (step 4)

The owner chose this route on 2026-11-15, after the preflight named LoadPin. A systemd **soft-reboot** stops every program above the kernel and starts another system's init on the kernel that is already running. It loads no kernel, so LoadPin's `kexec_load` refusal does not apply. The same three files as step 3 carry it (`hook.sh`, the modprobe line, the udev rule, same marker line), so `--hook-install` replaces the kexec version in place and `--hook-remove` is still the undo. Only the jump changed; the trigger, the once-per-boot marker, the 15 s limit and the firmware-updater wait are step 3's.

### What the jump service does at power-on

1. Finds the partition named `mpfree-test-root` (partition 1 of a USB disk). If Akai's system has already mounted it, it stops: not this boot.
2. Mounts it at `/run/nextroot`, `ro,noload`: read-only, and no journal replay, so nothing is written to the stick. Unlike step 3's copy it does not add `noexec`, `nodev` or `nosuid`, because this becomes the root of the next system. If anything is at `/run/nextroot` already, it stops and leaves it alone.
3. Checks that the stick holds a system it may start:
   - `/boot/mpfree-softreboot/PROTOCOL` is `mpfree-softreboot-1`, and `release` is the running kernel's release (`6.18.44-az01-2026-08-10-rt6`), so the stick's userspace was built for this kernel;
   - every file in `/boot/mpfree-softreboot/SHA256SUMS` matches, and the list includes the three files of the switch-off service (below), which must be enabled in `sysinit.target.wants`;
   - `/usr/lib/systemd/systemd` is there and executable, and the stick has an `os-release` file. systemd 255 switches to `/run/nextroot` only if it holds one (`path_is_os_tree()` in `src/core/main.c`); without it the soft-reboot would restart Akai's own system.
4. Waits up to 20 s while a firmware updater runs, as step 3 does, except for `az01-usbsata-fixer`: it runs all the time, so it counts only while a JMicron USB-SATA bridge (USB vendor `152d`) is attached, the only device it writes to (an assumption from its name, its log line "USB-SATA bridge fixer" and its unit; its code was not read). See the first power-on below.
5. Loads the kernel modules the stick lists in `preload-modules.txt`, if any, **from Akai's own system** (`modprobe` from the eMMC's trusted root). After the soft-reboot nothing can load a module or firmware at all (see below), so this is the only way to add one.
6. Writes `/run/mpfree-hook/soft-reboot` (which the stick's system reads) and starts `soft-reboot.target` (`systemctl --no-block --job-mode=replace-irreversibly start soft-reboot.target`, what `systemctl soft-reboot` does, without asking logind).

On any problem it notes `STOP: <reason>`, unmounts `/run/nextroot` and removes it, and exits. It must leave nothing at `/run/nextroot`, because systemd turns a later plain `reboot` into a soft-reboot when that folder holds a system. Akai's app then starts as usual.

What systemd 255 does next, read in its source (`units/systemd-soft-reboot.service`, `src/core/main.c`, `src/shared/switch-root.c`, `src/shared/fstab-util.c`):
- It stops all units, Akai's app among them, which never got past the jump service's `Before=`. `umount.target` unmounts the mounts it manages. `/run/nextroot` is left mounted, because systemd treats it as "extrinsic" (`fstab_is_extrinsic()`).
- It syncs, moves `/dev`, `/proc`, `/sys` and `/run` (not the mounts below `/run`) into `/run/nextroot`, pivots into it, and detaches the old root with `MNT_DETACH`. So `/run` and its files survive; the hook's notes stay readable.
- It execs `/usr/lib/systemd/systemd` from the stick, which boots the stick's system. The kernel, its command line and its boot ID stay Akai's.
- `/data` cannot be unmounted cleanly by `umount.target`, because Akai's `/etc` overlay (which systemd does not unmount) keeps its upper folder open. It is released when the old root is detached, after systemd's sync. The kernel finishes the unmount once nothing uses it.

### The stick's side: the eMMC and SD card are switched off first

After a kexec jump the stick's kernel had a device tree with the eMMC and SD controllers disabled. After a soft-reboot the kernel and device tree are Akai's, and both controllers are on. So `offline-kexec-stick.py` now adds `mpfree-test-internal-off.service`. It is enabled in `sysinit.target`, with `DefaultDependencies=no` and ordered before udev, every mount (`local-fs-pre.target`, `data.mount`) and MPFree's own services. It runs `/etc/mpfree-test/internal-off.sh`, which:

1. waits up to 10 s until no writable filesystem from the eMMC or SD card is still known to the kernel, even a detached one. It reads `/sys/fs/ext4/*`; a `dm-*` entry counts if it sits on `mmcblk*` and is not read-only. Akai's dm-verity root is read-only and is not waited for;
2. marks every `mmcblk*` device read-only (`blockdev --setro`, where the stick has it);
3. unbinds the driver of the eMMC controller (`ff0f0000.*`) and the SD controller (`ff0c0000.*`), so the kernel removes their block devices. The SDIO controller at `ff0d0000` is left alone;
4. checks that both have no driver and no `mmcblk*` device is left, and removes stale `/dev/disk/by-*` links to them (`/dev` comes over from Akai's boot).

If any step fails, it saves its notes and the kernel log to `boot-<boot id>/internal-off.txt` on the stick's own log partition (found by its GPT name `mpfree-test-log`, on USB), then exits 1. The unit's `FailureAction=poweroff` turns the MPC off. MPFree never runs with the internal storage reachable. A udev rule (`/etc/udev/rules.d/00-mpfree-internal-off.rules`) also sets `SYSTEMD_READY=0`, `UDISKS_IGNORE=1` and `UDISKS_AUTO=0` on any `mmcblk*` device that appears anyway. After a kexec jump there is nothing to unbind, and the service just notes that it ran.

The builder writes `/boot/mpfree-softreboot/` with the switch-off service's three files, `PROTOCOL`, `release`, optional `preload-modules.txt` (`--preload-module NAME`, repeatable) and their `SHA256SUMS`. The kexec files in `/boot/mpfree-kexec/` stay, so one stick serves both routes.

### LoadPin after the soft-reboot

LoadPin stays on across the soft-reboot. It is pinned to the superblock of Akai's dm-verity root, and that root is detached at the switch. When the pinned superblock is freed, LoadPin in enforcing mode refuses **every** later module, firmware and kexec load that does not come from a trusted verity device (`loadpin_sb_free_security()`: "umount pinned fs: refusing further loads"). So the stick's system:
- keeps the modules Akai's boot loaded in its first 6 s (udev's start-up pass loads the drivers for the hardware present, at about 3 s) and those the hook loads first;
- cannot load a module from its own `/lib/modules`. `systemd-modules-load` and udev's module loading will log refusals; that is expected;
- cannot load firmware. Firmware already in a chip stays. A driver that needs firmware after 6 s, such as the Wi-Fi driver when the interface is brought up, fails. Wi-Fi is not needed for the first release.

### Rehearsed in the simulator

- [`test_boot_hook.py`](../../../tools/mpc-one/tests/test_boot_hook.py) (18 tests) runs the wrapper, the installer, the undo and the report against the fake MPC. The fake stick is made from the builder's own constants, so the two agree.
  - The installer replaces the canary and the kexec version installed on the MPC now, checks the stick read-only (also through a real `nosymfollow` mount, as Akai's `edisksd` makes one) and leaves nothing in `/run`.
  - The jump mounts `/run/nextroot` `ro,noload`, loads the listed modules, writes the marker and asks for `soft-reboot.target` exactly once.
  - It stops and cleans up on every check above: a damaged or missing switch-off file, a stick for another kernel, another protocol, an older stick, a path out of the stick in `SHA256SUMS`, no systemd, no `os-release`, a missing module, the partition already mounted, something at `/run/nextroot` already, firmware updaters, and systemd refusing the soft-reboot.
- [`test_offline_kexec_stick.py`](../../../tools/mpc-one/tests/test_offline_kexec_stick.py) checks the new files on a built stick (the rule passes `udevadm verify`, systemd 255), the `--preload-module` checks, the `no-reserved-blocks@` mask, and the report's list of the units that udev rules start.
- It runs `internal-off.sh` under dash against a fake kernel, whose unbind file removes the driver and block devices the way Linux does. The tests cover:
  - the switch-off after a soft-reboot (the SDIO controller and the stick's links left alone);
  - the wait for a still-mounted `/data`, then the power-off without unbinding;
  - a controller that stays bound;
  - the note saved on the stick's log partition;
  - nothing to do after a kexec jump.

### Unknowns before the first power-on, answered by the second

- Akai's kernel command line and Akai's `/run` left over: the stick's system booted on both without trouble.
- Modules and firmware: everything the pad player needs (sound card, MIDI, Ethernet) was loaded by 6 s. After the switch the kernel logged LoadPin's "refusing further loads" and no refusal of anything needed, so the stick needs no `--preload-module`.
- Akai's `/data` was unmounted cleanly by the soft-reboot (7.76 s), so the switch-off did not have to wait.
- The pad player started 16.3 s after power-on. What the screen shows between the MPC logo and MPFree's startup screen (9.7 s) is not recorded yet.

### First power-on (2026-11-15, zip 090837Z): back to Akai's system

The install and its check passed (`CHECK-OK`, stick read at `/media/<UUID>`), but at the power-on with the stick in Akai's system started as usual. MPFree's system never ran: the stick's log partition holds no boot and no `internal-off.txt`. The notes of that boot were lost, because the report was taken after a power-off without the stick (`/run` and Akai's journal are in RAM).

The likely cause is in the install's own output: `note: az01-usbsata-fixer is running now`, 7,800 s after power-on. That program runs all the time on 3.10.1 (the [M7 record](../milestones/m07-validation-record.md) also lists its service as active). The jump counted any running `az01-usbsata-fixer` as a firmware writer, so it waited 20 s, stopped (`... is still running after 20 s`) and left Akai's app to start about 20 s late. The fixer is now waited for only while a JMicron bridge is attached; a test reproduces the old stop and checks the new behaviour. Step 3 had the same wait but never reached a power-on jump. Block 4.4 now takes the report while that boot is still running, so a second fall-through keeps its notes.

### Second power-on (2026-11-15, zip 092220Z): MPFree started

The owner reinstalled the hook (c00dfe44) and powered on with the stick in: the MPC went from the logo to MPFree's pad player with no tap, and the pads played. From the stick's `dmesg.txt` and `journal.txt` (kernel uptime):

| Time | What happened |
| --- | --- |
| 6.05 s | the modprobe line starts the jump; at 6.15 s the jump service runs, with Akai's app `inactive` |
| 6.59 s | the stick checks out; `soft-reboot.target` is started |
| 7.6 to 8.0 s | Akai's units stop; the content (p7) and data (p8) partitions unmount cleanly, then the root (p6) |
| 8.02 s | `LoadPin: umount pinned fs: refusing further loads`, as predicted |
| 8.32 s | the stick's systemd 255 starts |
| 9.68 s | `mpfree-test-internal-off.service` starts; the startup screen starts at 9.70 s |
| 9.82 s | `switched off ff0f0000.dwmmc` (the eMMC) |
| 13.34 s | `switched off ff0c0000.dwmmc` (the SD card; the controller was busy, see below) |
| 13.78 s | `done: no eMMC or SD card block device is left`; then udev (14.07 s) and the mounts |
| 16.34 s | the pad player starts; startup finished at 18.59 s |

- No eMMC filesystem was mounted in MPFree's boot. The boot-area hashes (p1 to p6, boot0, boot1) are the same in all three boot maps: before and after the first power-on, and after this one.
- The only failed unit is Akai's `az0x-loadpin-trustpoints`. After the switch LoadPin refuses the file it loads. That failure is harmless.
- The hook report with the stick in was refused (`Connection refused` on port 22). That was expected, because MPFree was running, not Akai's system. It is needed only after a fall-through.

**Found: one of Akai's units read the SD card before the switch-off finished.** A 64 GB exFAT card was in the slot. Akai's `no-reserved-blocks@.service` runs `set-no-ext4-reserved-blocks`, which runs `tune2fs` on ext4 partitions as they appear. It started for the card at 9.71 s, in parallel with the switch-off. The likely reason is that the stick's systemd takes over Akai's device units, and the units they want, from the udev database left in `/run`. What exactly pulled it in is not confirmed: Akai's rules were not read. It read the card while the controller was going away: two `Buffer I/O error on dev mmcblk1p1, logical block 2` at 13.30 s, then `ignoring  filesystem` with no type. Nothing was written. The tool acts only on ext4, and it could not even read the type. Had the card been ext4 and still readable, `tune2fs` would have written its superblock. The eMMC's instance (p7) did not start in this boot. Fix:
- the stick now masks `no-reserved-blocks@.service`, which masks every instance;
- the build report lists every unit that Akai's udev rules start when a device appears, each marked masked or NOT masked, so the next build shows whether anything else can start in that gap. The journal showed no other unit started before 13.78 s that reads a disk.

The install's check also printed `note: Akai's controller update.sh is running now`. That was the check's own text: it runs as `sh -c "<hook>"`, so the pattern matched its own command line. The pattern is now `update[.]sh`, and a test reproduces the false note.

### Blocks for the owner

Each block was rehearsed in the simulator at these paths as far as a simulator can: the wrapper modes, the installer, the check, the jump, the undo and the report through the fake SSH; the builder against a fake image. The MPC runs Akai's 3.10.1 with developer mode on, on the direct Ethernet cable.

Every run on port 22 checks developer mode's host key strictly against `~/.ssh/known_hosts`. If it is not recorded there yet (the runs before 2026-11-15 kept it in `~/mpfree-work/mpc-devmode-known_hosts`), run [Block Q1](firmware-3.10.1-qualification.md) once first: `--devmode-trust`, read-only on the MPC, which records it after you type `TRUST`. Without it, block 4.3 stops before anything runs on the MPC and says to run that block.

**Block 4.1: update, check, build** (workstation only; nothing contacts the MPC):

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin main:refs/remotes/origin/main && git switch --no-track -C softreboot origin/main && git log --oneline -1 && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_kexec_stick.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_boot_hook.py && python3 tools/mpc-one/tests/test_write_test_stick.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run build-arm -- bash -c 'export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" && cargo build --manifest-path rust/Cargo.toml --release --locked --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log -p mpfree-startup -p mpfree-app' && python3 tools/mpc-one/results.py run build-stick -- python3 tools/mpc-one/offline-kexec-stick.py --splash rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-startup --app rust/target/armv7-unknown-linux-gnueabihf/release/mpfree
```

The build report says `for MPFree's soft-reboot hook: /boot/mpfree-softreboot on partition 1 (protocol mpfree-softreboot-1, kernel release 6.18.44-az01-2026-08-10-rt6)`. The builder uses the newest `--kexec-preflight` folder, the one from the installed 3.10.1 (zip 082152Z).

**Block 4.2: write the stick** (**state-changing, the stick only**: erases its partitions 1 to 3, keeps partition 4 with the pad player's settings; asks for `WRITE`). Undo: write an older build to the stick the same way.

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run write-stick -- python3 tools/mpc-one/write-test-stick.py
```

**Block 4.3: boot map before, then install the hook with the stick in the MPC** (the boot map is read-only; the install is **state-changing, the data partition only**: it rewrites the three hook files, then checks the stick read-only). Put the stick in the MPC while Akai's system runs first.
- Undo: `cd /home/jesse/code/mpfree-hardware-survey && MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --hook-remove`. It deletes the three files and the folders the installs made. By hand: `rm /data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf /data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules; rm -r /data/mpfree-hook`.
- If it is cut off halfway: each file is written under a temporary name and renamed, the script first, so nothing points at a half-written script. Run it again to finish.

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run boot-map-before -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --boot-map && python3 tools/mpc-one/results.py run hook-install -- env MPC_PORT=22 MPFREE_ACCEPT_HOOK=YES bash tools/mpc-one/collect-over-ssh.sh --hook-install
```

It must end with `RESULT: the hook is installed, and the stick checks out.` If it stops instead, take the stick out before the next power-on and send the results.

**Step 4.4: the power-on** (**state-changing**: Akai's programs are stopped at about 6 s and MPFree runs from the stick; the eMMC is switched off in software, nothing is written to it). Switch the MPC off, wait 10 s, and switch it on with the stick in. Expected: the MPC logo, then MPFree's startup screen and the pad player, with Akai's app never drawing. Play a few pads.
- **If Akai's app comes up instead, leave the stick in and the MPC on**, and run this read-only block first. The hook's notes are in RAM and only last until the power-off:

  ```bash
  cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-report-stick-in -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --hook-report
  ```

- If the MPC switches itself off, the switch-off service refused to go on.
- If the screen stays on the logo for more than a minute, hold the power button to switch off.
- Undo, always: switch off, take the stick out, wait 10 s, switch on. Akai's system starts as before. Don't touch the MPC's power while it shows an update screen.

**Block 4.5: what happened** (read-only). Switch off, take the stick out, wait 10 s and switch on into Akai's system. Then:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run hook-report -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --hook-report ; python3 tools/mpc-one/results.py run boot-map-after -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --boot-map
```

Then put the stick in the workstation:

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run stick-logs -- python3 tools/mpc-one/read-stick-logs.py ; python3 tools/mpc-one/results.py zip
```

**Passes when** the MPC went from the logo to MPFree with no tap and Akai's app never drew, and the stick's `dmesg.txt` shows `mpfree-internal-off: switched off ff0f0000...` before any `sd`/`mmc` mount. Nothing else may read the eMMC or SD card before `mpfree-internal-off: done`: no `no-reserved-blocks` unit and no `Buffer I/O error` on `mmcblk`. The boot maps before and after must have the same boot-area hashes, with 0 sectors written to p1 to p6 and boot0/1.

## Next steps

1. The runtime-kexec route is closed on 3.10.1: LoadPin, enforcing and pinned to the dm-verity root, refuses the stick's `kexec_load` (`kexec-image old-api-denied`; preflight zip 082152Z). IPE is absent and lockdown is off.
2. The owner chose soft-reboot. Step 4 works: the second power-on went from the MPC logo to MPFree with no tap (zip 092220Z). Next: rebuild and rewrite the stick with the `no-reserved-blocks@` mask (blocks 4.1 and 4.2), install the hook again (block 4.3, for the corrected check), and power on with the SD card in (4.4 and 4.5). That run passes when nothing reads the card before the switch-off is done, and the build report lists no other unit that is NOT masked and could touch a disk.
3. The **bootloader-level** route (U-Boot booting MPFree directly, or a kernel command-line word that lifts LoadPin) stays behind the [safety rule](../../agents/README.md): proven MaskROM recovery and the owner's go. Not started.
4. The hook needs installing once on each MPC. Whether a one-time tap at install is acceptable is the owner's decision; the owner rejected a tap at every power-on.
5. After any Akai update, rebuild the stick from a fresh `--kexec-preflight`. Its `release` must match the running kernel, or the hook refuses the stick and Akai's system starts.
