# Starting MPFree from the stick at power-on

**As of 2026-11-15** · **Device:** the reference MPC One, Akai MPC OS 3.10.1 installed on the eMMC (kernel `6.18.44-az01-2026-08-10-rt6`), developer mode on.

## The goal

The owner's goal for the first release (2026-11-14, sharpened 2026-11-15): with the MPFree stick in, switching the MPC on starts MPFree with no computer and no tap. The MPC logo may show for a moment, but Akai's app must not load first; MPFree's startup screen is the first thing after the logo. With the stick out, the MPC starts the factory system as before.

The jump itself is the kexec that [the stick test](firmware-3.10.1-stick-test.md) already uses: Akai's Linux loads the kernel from the stick with `kexec --load` and `systemctl kexec` starts it, with a device tree that hides the eMMC and SD controllers. What is missing is something that starts that jump at power-on, early enough.

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

Nothing here touches the root filesystem, the boot partitions, the U-Boot environment or the rest of `/etc`. The first run on port 22 saves the MPC's SSH host key in `~/mpfree-work/mpc-devmode-known_hosts` (behind the wrapper's MAC check); every later run must see the same key.

**Passes when**, after a power-on with a stick in, the report shows a `service (...) ran` line at a time before Akai's app draws its screen, which proves the path a jump would take, and the `modprobe loads cfg80211` line, which shows whether the module loader is the sturdier trigger.

**First run (2026-11-15, first version):** the install worked (both files written and read back, in the three folders it made), but the MPC was not switched off and on afterwards: it had been on for 646 s, and the stick went in 30 s after the install. As expected, nothing fired.

**First power-on (2026-11-15, first version, the MPFree test stick in):** the remove and a new install both worked on the device. After the power-on `/etc/udev/rules.d` showed the rule, and udev ran it from 6.42 s: `add` events for the module `cfg80211`, its class `ieee80211` and the device `regulatory`, 0.1 s before Akai's app was started (6.52 s). The next event was a backlight `change` at 17.02 s. The stick itself appeared at 2.79 s, before the rule was loaded, so the `udev saw` line and its service never ran: the path from a rule to a service at boot is still unproven. Timeline: `udevd` 3.11 s, start-up pass ended 3.25 s, `/data` 4.28 s, `/etc` 4.40 s, edisksd mounted the stick at 16.01 s. The second version's first report was again taken without a restart (12 s after the install); it showed who loads `cfg80211`: Akai's network service, connman, started at 6.14 s and the module loaded at 6.35 s.

**Second version, power-on (2026-11-15, the spare FAT stick in):** it passes. Both files showed in `/etc` after the power-on.
- The modprobe line ran at 5.96 s, called by the kernel itself (`sh < modprobe < kworker < kthreadd`), and the service it started ran at 6.09 s, with Akai's app not yet started. systemd logged `Started MPFree canary (modprobe)` at 6.07 s; Akai's app (`Started InMusic MPC Application`) followed at 6.36 s. connman had started at 5.78 s; `cfg80211` finished loading at 6.22 s, so the line held it up by about 0.25 s, which nothing noticed.
- udev's first event after loading the rule came at 6.28 s (the `add` events of `cfg80211`), and its service ran at 6.43 s, by when Akai's app was already running. Later events: the network queues' `remove` events at 11.3 to 11.4 s and the backlight `change` at 17.28 s.
- The service saw these partition labels: `content data env recoverysplash rootfs splash uboot uboot-spl` (the eMMC's; the spare stick has no GPT labels). The MPFree stick's labels (`mpfree-test-root` and so on) are set by udev at its start-up pass, before 3.3 s, so a hook can look for them at 6 s.

## The hook (step 3)

Built on what the second canary showed. Three more modes of [`collect-over-ssh.sh`](../../../tools/mpc-one/collect-over-ssh.sh), on port 22 only:

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
   - mounts it read-only (`ro,noload,nodev,nosuid,noexec`; or reads it where Akai's system already mounted it) and copies `/boot/mpfree-kexec` to `/run/mpfree-hook/kexec`: `zImage` (which must be a link to `../zImage-<release>` on the stick), `kexec.dtb`, `cmdline.txt` and `SHA256SUMS`, each with a size limit, then unmounts it;
   - checks the copies against `SHA256SUMS`, and that the command line is one line of plain characters that starts the system on this very partition (`root=PARTUUID=` its UUID) with `ro`, `rootwait=60`, `panic=10` and `sysctl.kernel.kexec_load_disabled=1`;
   - waits, up to 20 s, while a firmware updater runs (one of those units activating, `dfu-util`, `touch-fw-upd`, `xmosdfu`, `midifirmup`, `az01-update`, `az01-usbsata-fixer` or Akai's `SME0/Firmware/update.sh`); if one is still running it gives up, since a restart must never cut a firmware write short;
   - loads the kernel (`/usr/sbin/kexec --load --kexec-syscall --dtb=… --command-line=…`), checks `kexec_loaded` is 1 and that no updater has started, and starts `kexec.target` (`systemctl --no-block --job-mode=replace-irreversibly start kexec.target`, what `systemctl kexec` does, without asking logind, which may not be running yet). systemd stops what has started (Akai's app, if it has), unmounts `/data` cleanly and starts the stick's kernel.

   On any problem it notes `STOP: <reason>` in the kernel log and `/run/mpfree-hook.txt`, unloads a loaded kernel, unmounts the stick, deletes its copies, and exits; Akai's app then starts as usual.

The stick carries the jump's files since this change: [`offline-kexec-stick.py`](../../../tools/mpc-one/offline-kexec-stick.py) adds `/boot/mpfree-kexec/` to partition 1 (the same `kexec.dtb` and `cmdline.txt` as in the build folder, a link to the image's own kernel, and their `SHA256SUMS`). It also builds for an MPC that runs the stick's own release, which is the installed 3.10.1 starting the 3.10.1 stick: then the running tree from `--kexec-preflight` is read against the same release's board file, and must be the installed system's (its eMMC controller enabled), not a tree read on a test build started by kexec. The device tree carries this unit's serial number and MAC address, as the build folder's always has: the stick stays private.

Tests: [`test_boot_hook.py`](../../../tools/mpc-one/tests/test_boot_hook.py) runs the wrapper, the installer, the undo and the report against a fake MPC, and the hook's triggers under BusyBox's shell; the jump and the check run under dash (BusyBox's shell runs its own `mount`, `sleep` and `pidof` before any stand-in) with stand-ins for `mount`, `kexec`, `systemctl` and `systemd-run`, through every way it can stop.

What is still unknown until the first power-on: whether `systemd-run` queues the service before Akai's app starts (the canary's service was queued about 0.3 s before it), how long the restart takes, and what the screen shows between the MPC logo and MPFree's startup screen.

## Next steps

1. On the MPC (blocks in the thread, rehearsed first): `--kexec-preflight` on port 22 (read-only), a stick build for the installed 3.10.1, `write-test-stick.py`, `--hook-install` with the stick in, then a power-on with the stick in (MPFree's startup screen should come after the logo, without Akai's app) and one without it (Akai's system, and `--hook-report`).
2. For the release: the hook needs installing once on each MPC. The stick's `autorun.sh` could do it with the one tap the first time (Akai's script runner asks); the owner rejected the tap at every power-on, and whether one tap at install is acceptable is the owner's decision.
3. After any Akai update: rebuild the stick from a fresh `--kexec-preflight`, since the device tree on the stick comes from the running system's.
