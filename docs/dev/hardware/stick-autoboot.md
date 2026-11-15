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
| A **udev rule** in Akai's writable `/etc` layer | Maybe | Yes | `/data` | Being tested (the canary below) |

### What the script runner test showed (2026-11-15)

A spare FAT32 stick with the harmless `autorun.sh` from [`write-autorun-drive.py`](../../../tools/mpc-one/write-autorun-drive.py), in the MPC at power-on: the MPC asked only once Akai's app was fully up. `az01-launch-MPC` logged `About to run unprotected script: /media/USB321FD/autorun.sh`, then ran it through `systemd-run` as a transient `autorun-sh.service` in `system.slice` (not in the app's cgroup), as root (`uid=0`), 26 s after power-on, with `/usr/sbin` on its `PATH`. The FAT32 stick was mounted `noexec` at `/media/<label>`. kexec is allowed on the installed system: `kexec_load_disabled` is 0, nothing is loaded, `/usr/sbin/kexec` ships; `/sys/kernel/security/lockdown` does not exist.

Akai's `/etc` is two overlays: `etc.mount` (lower `/etc`, upper `/data/system/etc/overlay`) and developer mode's read-only one on top (lower `/usr/lib/az0x/user-access/etc-overlay:/etc`). The upper still holds `mpfree-sshd.service` and its `multi-user.target.wants` link from the November 9 attempt on 3.6.0. They have never run at boot, for the reason above, and are harmless.

## Why a udev rule might work

udev is the part of Linux that reacts when a device, such as a USB stick, appears. Unlike systemd's unit folders, udev re-reads its rule folders while it runs. In systemd 255 (3.10.1 has 255 or later), `udevd` checks the `.rules` files in `/etc/udev/rules.d` and the other rule folders each time it starts queued events, at most every 3 s (`manager_reload()` and `udev_rules_should_reload()` in `src/udev/udev-manager.c` and `udev-rules.c`). Once `etc.mount` has put the writable layer over `/etc`, a rule kept there is seen from the next check on.

The timing decides whether it helps: the stick may appear, and its events may be handled, before `/etc` is mounted or before udev's next check. Akai's app program starts at about 6 s and draws its screen at about 20 s. A rule must also not do the jump itself: udev runs its programs under a system-call filter without `kexec_load` or `mount`, so a real hook asks systemd to start a short service (`systemd-run --no-block`) that does the work, the way Akai's launcher starts `autorun.sh`.

## The canary (step 1)

Three modes of [`collect-over-ssh.sh`](../../../tools/mpc-one/collect-over-ssh.sh), on developer mode's port 22 only (`MPC_PORT=22`):

- `--hook-canary` (**state-changing, data partition only**; needs `MPFREE_ACCEPT_HOOK_CANARY=YES`) streams [`hook-canary-install.sh`](../../../tools/mpc-one/hook-canary-install.sh). It checks that the MPC runs 3.10.1 (kernel 6.18, az01) with `/etc` as the overlay above, then writes two files and the folders they need, and records which folders it made:
  - `/data/mpfree-hook/canary.sh`
  - `/data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules`

  The rule runs `canary.sh` for every USB disk partition that appears. The script writes one line to the kernel log and to `/run/mpfree-canary.txt` (RAM): the time since power-on, whether `acvs.service` and the `MPC` program are running, and the partition's name. It then asks systemd to run itself once more as a short service, which logs a second line: this proves the path a real hook would use. A second rule line notes every other device event (add or change) in `/run/mpfree-canary.txt`, which shows when udev loaded the rule and which later events a hook could wait for. Nothing else is read, written, mounted, loaded or started. A rule udev cannot parse is skipped with a log message and cannot stop the MPC from booting; the rule passes `udevadm verify` (systemd 255).
  If the write is cut off: the script is written first and the rule last, each under a temporary name udev ignores and then renamed, so the rule never points at a missing script. Running it again finishes the job, or reports that both files are already in place.
  It writes the upper folder directly, since the `/etc` that programs see is developer mode's read-only overlay. The overlay already mounted may or may not show the new files before the next power-on; nothing breaks either way.
- `--hook-canary-report` (read-only) streams [`read-only-hook-canary.sh`](../../../tools/mpc-one/read-only-hook-canary.sh): the canary's files and hashes, its lines in the kernel log and `/run`, and this boot's timeline from the journal (when `/data` and `/etc` were mounted, when the stick appeared, when Akai's app was started).
- `--hook-canary-remove` (**state-changing, data partition only**, the undo) streams [`hook-canary-remove.sh`](../../../tools/mpc-one/hook-canary-remove.sh). It deletes the two files only if they carry the canary's marker line, and the folders the install made only if they are empty. Running it again is harmless. By hand: `rm /data/system/etc/overlay/udev/rules.d/99-mpfree-canary.rules; rm -r /data/mpfree-hook`.

Nothing here touches the root filesystem, the boot partitions, the U-Boot environment or the rest of `/etc`. The first run on port 22 saves the MPC's SSH host key in `~/mpfree-work/mpc-devmode-known_hosts` (behind the wrapper's MAC check); every later run must see the same key.

**Passes when** the report shows the canary's `udev saw sdXN` line, and its `service saw sdXN` line, at a time before `acvs.service` started, after a power-on with a stick in. If the line comes only after the app started, or only for a stick plugged in later, the timeline shows which events come late enough to wait for instead.

## Next steps

1. If the canary fires early enough: a hook that does the same for the MPFree stick only (matched by its partition), starts a short service that checks the stick and its files, copies the kernel and device tree into RAM, `kexec --load`s them and runs `systemctl kexec`. The stick test's kexec build provides the kernel, device tree and root; it needs a fresh `--kexec-preflight` over port 22 first, for the installed 3.10.1's running device tree.
2. For the release: the stick's `autorun.sh` installs the hook once (one tap, the first time only); from then on every power-on with the stick in goes straight to MPFree.
3. If udev cannot act early enough: what remains needs the safety rule changed (the root filesystem or U-Boot), which is the owner's decision.
