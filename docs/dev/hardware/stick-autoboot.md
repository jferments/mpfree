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

## Next steps

1. The hook (the second canary passed): the modprobe line as the trigger, with the udev rule as a backup, whichever runs first, starting a short service only when the MPFree stick is in (its partition label). The service is ordered before `acvs.service`, so Akai's app waits for it; if the app has started anyway, the jump stops it before it draws anything (its screen comes at about 17 s). The service checks the stick and its files, copies the kernel and device tree into RAM, `kexec --load`s them and runs `systemctl kexec`; anything wrong, and it exits and the MPC starts Akai's app as usual. The stick test's kexec build provides the kernel, device tree and root; it needs a fresh `--kexec-preflight` over port 22 first, for the installed 3.10.1's running device tree.
2. For the release: the stick's `autorun.sh` installs the hook once (one tap, the first time only); from then on every power-on with the stick in goes straight to MPFree.
3. If udev cannot act early enough: what remains needs the safety rule changed (the root filesystem or U-Boot), which is the owner's decision.
