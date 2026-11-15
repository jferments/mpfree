# How MPFree boots from the stick on MPC OS 3.10.1

**As of 2026-11-15** · **Status:** works on the reference MPC One (Akai MPC OS 3.10.1 installed, kernel `6.18.44-az01-2026-08-10-rt6`); merged in #370 · **Record of the runs and the owner's blocks:** [Starting MPFree from the stick at power-on](stick-autoboot.md)

With the MPFree stick in at power-on, the MPC shows its logo and then MPFree's pad player, with no tap; Akai's app never draws. With the stick out, the MPC boots Akai's system exactly as before. This page explains how that works: the parts, the order they run in, what each one checks, and why it was built this way. The step-by-step history (the canary, the kexec hook that LoadPin refused, both power-ons) is in [the stick auto-boot record](stick-autoboot.md).

## In one paragraph

A small **hook** on Akai's writable data partition notices the stick about 6 s into Akai's boot, checks it, and asks Akai's own systemd for a **soft-reboot**: systemd stops every program and starts the stick's systemd as the new PID 1, on the kernel that is already running. No kernel is loaded, so the kernel's LoadPin policy, which refuses a kexec on 3.10.1, does not apply. Because Akai's kernel and device tree stay, the MPC's eMMC and SD controllers are still live after the switch, so the stick's **first service unbinds both of them** before udev or any mount runs, and powers the MPC off if it can't. Then MPFree starts.

## Constraints

| Rule | What it forced |
| --- | --- |
| No writes to the eMMC boot partitions, the U-Boot environment or Akai's root filesystem ([safety rule](../../agents/README.md)) | No U-Boot or kernel command-line change, no unit in the root filesystem. The hook lives only on `/data`. |
| Switch the internal storage off in software before anything touches it | The stick unbinds the eMMC and SD controllers first, and powers off on failure. |
| No kernel modules from the stick | The stick runs on Akai's kernel and the modules it already loaded. |
| Write only to the data partition, never to the stick | The stick's root is mounted `ro,noload`; the hook's power-on notes go to `/run` and the kernel log. |
| Never restart during a firmware update | The jump waits up to 20 s for Akai's firmware updaters and gives up if one still runs. |
| Stick out means stock boot | Without the stick the hook only notes it in `/run`. Taking the stick out is always the undo. |

## Why a soft-reboot and not a kexec

The first design was the stick test's kexec: Akai's Linux loads the stick's kernel and a device tree that disables the eMMC and SD controllers, then jumps. On the installed 3.10.1 the kernel refuses that load. **LoadPin** runs in enforcing mode, pinned to Akai's dm-verity root, and denies every raw `kexec_load` (`LoadPin: kexec-image old-api-denied`). Lifting it needs `loadpin.enforce=0` on the kernel command line, which lives in U-Boot and is off limits. IPE is not active on this unit and lockdown is not set ([details](stick-autoboot.md#the-hook-step-3-the-kexec-version-replaced)).

A systemd 255 soft-reboot loads no kernel. It stops all units, pivots into a new root at `/run/nextroot` and execs that root's `/usr/lib/systemd/systemd`. Akai's kernel, device tree and command line stay; everything above the kernel becomes MPFree's.

| Route | Result | Why |
| --- | --- | --- |
| Akai's script runner (`autorun.sh` on a drive) | Rejected | It asks for a tap, and only after Akai's app is up. |
| A systemd unit in Akai's writable `/etc` layer | Ruled out | systemd reads its unit folders before `etc.mount` lays `/data` over `/etc`. |
| A systemd unit in the root filesystem | Blocked | Root filesystem write; an Akai update would remove it. |
| U-Boot booting USB first | Blocked | U-Boot environment write, behind proven MaskROM recovery. |
| Raw `kexec_load` from the hook | Refused by the kernel | LoadPin. |
| `kexec_file_load` of Akai's own signed kernel | Parked | `--dtb` is refused in file mode, so it gives Akai's kernel and device tree with our root: what a soft-reboot gives, plus a full kernel restart. Never tried as a jump. |
| MPFree under Akai's init (chroot or namespace) | Fallback | Akai's systemd stays PID 1 and its units can still write `/data`. |
| **systemd soft-reboot** | **Chosen, works** | No kernel load, Akai's userspace fully stopped, MPFree's systemd as PID 1. |

The price is that the internal storage is no longer hidden by a device tree, so the stick must switch it off itself, and that LoadPin refuses every module and firmware load after the switch ([below](#what-survives-the-soft-reboot)).

## The boot, end to end

Seconds since power-on, from the working power-on of 2026-11-15:

```text
Akai's system, from the eMMC (read-only root)
  0 s      BootROM, U-Boot, Akai's kernel                 unchanged
  3 s      Akai's systemd, udev                           labels the stick's partitions
  4.2 s    /data, then /etc (overlay with /data upper)    the hook's two trigger files appear
  6.05 s   connman asks for cfg80211
           -> modprobe 'install' line runs hook.sh        no stick: note it, stock boot
           -> systemd-run mpfree-jump (Before= Akai's app and updaters)
  6.15 s   mpfree-jump: mount stick ro,noload at /run/nextroot,
           check the manifest, wait for firmware updaters  any problem: STOP, clean up,
                                                            Akai's app starts as usual
  6.59 s   systemctl start soft-reboot.target
  7.6-8.0 s Akai's units stop; content, data, root unmount
  8.02 s   LoadPin: "umount pinned fs: refusing further loads"
MPFree's system, from the stick (read-only root), same kernel
  8.32 s   the stick's systemd 255 is PID 1
  9.68 s   mpfree-test-internal-off.service starts        fails: the MPC powers off
  9.70 s   startup screen (framebuffer only, in parallel)
  9.82 s   eMMC controller ff0f0000 unbound
  13.34 s  SD controller ff0c0000 unbound
  13.78 s  no eMMC or SD block device left; then udev and mounts
  16.34 s  pad player
```

## The hook on Akai's data partition

[`hook-install.sh`](../../../tools/mpc-one/hook-install.sh) writes three files, each with a marker line so that [`hook-remove.sh`](../../../tools/mpc-one/hook-remove.sh) deletes only its own. Akai's `/etc` is an overlay whose writable upper layer is `/data/system/etc/overlay`, so two of them appear in `/etc` from the next power-on without touching the root filesystem.

| File | Role |
| --- | --- |
| `/data/system/etc/overlay/modprobe.d/99-mpfree-hook.conf` | **Main trigger.** `install cfg80211 ... /bin/sh /data/mpfree-hook/hook.sh modprobe; /usr/sbin/modprobe --ignore-install cfg80211`. When the kernel asks for the Wi-Fi module, kmod runs the hook first, then loads the module as before. |
| `/data/system/etc/overlay/udev/rules.d/99-mpfree-hook.rules` | **Backup trigger.** Runs `hook.sh udev` on a device event once udev has re-read its rules, only while `/run/mpfree-hook` does not exist and a partition named `mpfree-test-root` does. |
| `/data/mpfree-hook/hook.sh` | The logic, in three modes: trigger, jump and check. |

**Why these triggers.** systemd never sees a unit in the `/data` layer, but udev and kmod re-read their configuration while they run. kmod reads `/etc/modprobe.d` on every call, so the `install` line fires the moment `cfg80211` is requested: 5.96 to 6.05 s on 3.10.1, about 0.3 s before Akai's app is started. udev re-reads its rules at most every 3 s, so its timing depends on when the next event lands; it is only a backup. The [canary](stick-autoboot.md#the-canary-steps-1-and-2) measured both before any hook existed. The modprobe line names kmod's `/usr/sbin/modprobe` by full path (BusyBox's ignores `install` lines), and it never waits on systemd, which may itself be waiting for the module.

**What `hook.sh` does.**

1. **Trigger** (`modprobe` or `udev` mode). Returns at once if `/run/mpfree-hook` exists (once per boot) or the uptime is past 15 s. Without `/dev/disk/by-partlabel/mpfree-test-root` it notes "no stick" in `/run/mpfree-hook.txt` and returns. Otherwise it makes `/run/mpfree-hook` and starts the jump with `systemd-run --no-block --unit=mpfree-jump` (`Type=oneshot`, `TimeoutStartSec=45`, `Before=acvs.service touch-fw-update.service xmos-update.service az01-usbsata-fixer.service`). The `Before=` ordering holds Akai's app and its firmware updaters while the jump runs.
2. **Jump** (the `mpfree-jump` service):
   1. finds `mpfree-test-root`, which must be partition 1 of a USB disk; if Akai's system has already mounted it, stops;
   2. mounts it at `/run/nextroot` with `ro,noload` (no journal replay, so nothing is written to the stick); stops if anything is there already;
   3. checks `/boot/mpfree-softreboot/` on the stick: `PROTOCOL` is `mpfree-softreboot-1`; `release` equals the running kernel's release; every file in `SHA256SUMS` matches; the list includes the switch-off service's three files and the service is enabled in `sysinit.target.wants`; `/usr/lib/systemd/systemd` is executable; an `os-release` exists (systemd switches to `/run/nextroot` only when it holds an OS tree; without one a soft-reboot would restart Akai's system);
   4. waits up to 20 s while a firmware updater runs (`touch-fw-update`, `xmos-update`, `dfu-util`, `touch-fw-upd`, `xmosdfu`, `midifirmup`, `az01-update`, Akai's `SME0/Firmware/update.sh`). `az01-usbsata-fixer` runs all the time on 3.10.1, so it counts only while a JMicron bridge (USB vendor `152d`) is attached. Still running after 20 s: stop;
   5. loads the modules listed in `preload-modules.txt`, if any, from Akai's own trusted root (none are needed today);
   6. writes `/run/mpfree-hook/soft-reboot` and runs `systemctl --no-block --job-mode=replace-irreversibly start soft-reboot.target` (what `systemctl soft-reboot` does, without asking logind).
3. **Check** (run by the installer over SSH, with the stick in): the jump's steps 1 to 4 without the soft-reboot, ending in a `RESULT:` line.

On any problem the jump logs `STOP: <reason>` to the kernel log and `/run/mpfree-hook.txt`, unmounts and removes `/run/nextroot`, and exits; Akai's app then starts as usual, a little late. Leaving nothing at `/run/nextroot` matters, because systemd turns a later plain `reboot` into a soft-reboot when that folder holds a system.

## The stick

[`offline-kexec-stick.py`](../../../tools/mpc-one/offline-kexec-stick.py) builds the image on the workstation and [`write-test-stick.py`](../../../tools/mpc-one/write-test-stick.py) writes it. The stick's system is a **modified copy of Akai's own root filesystem** for the same release, so its userspace matches the kernel it runs on; MPFree's programs run on top of it. The builder never contacts the MPC and never writes a device.

| Partition | GPT name | Contents | Mounted as |
| --- | --- | --- | --- |
| 1 | `mpfree-test-root` | The modified root filesystem, `/boot/mpfree-softreboot/` (and `/boot/mpfree-kexec/` for the kexec route) | `/`, read-only |
| 2 | `mpfree-test-log` | Boot logs, copied every 10 s; `internal-off.txt` after a failed switch-off | written only by MPFree's log service |
| 3 | | Drum kits (`kits.bin`) | read-only |
| 4 | `mpfree-test-data` | The pad player's settings and files; kept when the stick is rewritten | `/run/mpfree-data`, read-write, `nosuid,nodev,noexec` |

What the builder changes in the copy, for this route:

- **masks** every Akai unit that could write, flash or format: Akai's app (`acvs.service`), the touch, XMOS and USB-SATA firmware updaters, the network update server, the USB script runner, the `/data` and content formatters, the firewall, the eMMC content generator, and `no-reserved-blocks@.service` (tune2fs; masking the template masks every instance);
- **stubs** Akai's update, flashing, U-Boot-environment and kexec programs with a program that refuses and logs;
- makes `/data` a RAM disk, so Akai's `/etc` and `/var` overlays live in RAM;
- adds key-only SSH on port 2222, the log service, the startup screen and the pad player;
- only with `--app-user` (opt-in, not yet tried on the MPC): runs the pad player as the `mpfree` user with no capabilities instead of root, after a root `mpfree-test-storage.service` mounts its partitions ([details](stick-autoboot.md#the-pad-player-without-root---app-user-opt-in)). Without the flag it runs as root, as before;
- adds the switch-off service and its udev rule (next section);
- writes `/boot/mpfree-softreboot/`: `PROTOCOL`, `release` (the kernel release the system was built for), optional `preload-modules.txt` (`--preload-module NAME`) and `SHA256SUMS` over those and the switch-off service's three files;
- prints a build report that lists every unit Akai's udev rules start when a device appears, each marked masked or **NOT masked**.

### Switching off the internal storage

`mpfree-test-internal-off.service` is enabled in `sysinit.target` with `DefaultDependencies=no`, ordered before udev, `local-fs-pre.target`, `data.mount` and every MPFree service, with `FailureAction=poweroff`. Its script `/etc/mpfree-test/internal-off.sh`:

1. waits up to 10 s until no writable eMMC or SD filesystem is still known to the kernel (`/sys/fs/ext4/*`, a writable `dm-*` on `mmcblk*` included; Akai's read-only verity root is not waited for);
2. marks every `mmcblk*` device read-only (`blockdev --setro`);
3. unbinds the drivers of the eMMC controller (`ff0f0000.*`) and the SD controller (`ff0c0000.*`), so the kernel removes their block devices; the SDIO controller at `ff0d0000` is left alone;
4. checks that both have no driver and no `mmcblk*` is left, and removes stale `/dev/disk/by-*` links (`/dev` comes over from Akai's boot).

If any step fails it saves its notes to the stick's log partition and exits 1, and systemd powers the MPC off: MPFree never runs with the internal storage reachable. A udev rule (`00-mpfree-internal-off.rules`) also sets `SYSTEMD_READY=0`, `UDISKS_IGNORE=1` and `UDISKS_AUTO=0` on any `mmcblk*` device that appears anyway. After a kexec jump there is nothing to unbind, and the service just notes that it ran.

## What survives the soft-reboot

| Survives | Consequence | How it is handled |
| --- | --- | --- |
| Akai's kernel, device tree, command line and boot ID | The eMMC and SD controllers are on; the stick's userspace must suit the kernel | The switch-off service; the hook's `release` check |
| Modules loaded in Akai's first 6 s | Sound card, MIDI and Ethernet drivers are already there | Nothing to do; no `--preload-module` was needed |
| LoadPin, with its pinned root gone | Once Akai's verity root is detached, LoadPin refuses **every** later module, firmware and kexec load | Extra modules only via `preload-modules.txt`, loaded by Akai's system before the switch. Wi-Fi firmware cannot load (not needed for the first release). Akai's `az0x-loadpin-trustpoints` fails, harmlessly |
| `/run`, including the udev database | The hook's notes stay readable. The stick's systemd also takes over Akai's device units and the units their udev rules want (`SYSTEMD_WANTS`), which can start on the eMMC or SD card before the switch-off ends | Every Akai unit that can touch a disk is masked; the build report lists all udev-started units |
| `/dev`, moved into the new root | Stale `/dev/disk/by-*` links to the eMMC | Removed by the switch-off script |
| `/run/nextroot`, left mounted (systemd treats it as extrinsic) | It becomes the new `/` | Mounted `ro,noload` by the hook |

Akai's `/data` does not survive: `umount.target` cannot unmount it while Akai's `/etc` overlay holds it, and the kernel releases it when the old root is detached after systemd's sync. On the working power-on the content and data partitions unmounted cleanly at 7.6 to 8.0 s.

## Owner tooling

All MPC-side steps go through [`collect-over-ssh.sh`](../../../tools/mpc-one/collect-over-ssh.sh) on Akai's developer-mode SSH (`MPC_PORT=22`; developer mode on with SHIFT + MENU > Developer). Every run checks the MPC's host key strictly against `~/.ssh/known_hosts`, recorded once by `--devmode-trust` ([qualification runbook, Block Q1](firmware-3.10.1-qualification.md)).

| Mode | Kind | Opt-in | Undo |
| --- | --- | --- | --- |
| `--hook-install`: checks the MPC, writes the three files (temporary name then rename, script first), removes any canary, then checks the stick if it is in | state-changing, `/data` only | `MPFREE_ACCEPT_HOOK=YES` | `--hook-remove` |
| `--hook-report`: the files and hashes, whether `/etc` shows them, whether the stick is in, the hook's notes and this boot's timeline | read-only | | |
| `--hook-remove`: deletes only files with the hook's or the canary's marker line, and the empty folders the installs made | state-changing, `/data` only | | `--hook-install` |
| `--hook-canary`, `--hook-canary-report`, `--hook-canary-remove`: the measuring predecessor; starts nothing | as above | `MPFREE_ACCEPT_HOOK_CANARY=YES` | `--hook-canary-remove` |

The owner's sequence is blocks 4.1 to 4.5 of [the record](stick-autoboot.md#blocks-for-the-owner): build, write the stick, install with the stick in, power on, report. The hook's notes are in RAM, so after a power-on that ends in Akai's system, run `--hook-report` before switching off. The universal undo is physical: switch off, take the stick out, switch on.

## Tests

- [`test_boot_hook.py`](../../../tools/mpc-one/tests/test_boot_hook.py) runs the wrapper, the installer, the jump, the check, the undo and the report against a fake MPC, the triggers under BusyBox's shell and the jump under dash, through every way the jump can stop.
- [`test_offline_kexec_stick.py`](../../../tools/mpc-one/tests/test_offline_kexec_stick.py) checks the builder's output (the rule passes `udevadm verify`), the masks, the udev audit in the report, and runs `internal-off.sh` against a fake kernel whose unbind file behaves like Linux's.
- [`test_hook_canary.py`](../../../tools/mpc-one/tests/test_hook_canary.py) covers the canary and the wrapper's host-key check.
- CI's shell syntax and static no-write checks cover these scripts.

## Limits and what to watch

- **Tied to the kernel release.** After any Akai update the hook refuses the stick (its `release` no longer matches) and Akai's system starts. Rebuild the stick from a fresh `--kexec-preflight`.
- **No module or firmware loads after the switch.** Anything MPFree needs must be loaded in Akai's first 6 s or listed in `preload-modules.txt`.
- **Inherited udev state.** Any Akai unit that a udev rule starts for a disk must be masked on the stick. Read the build report's udev list after every Akai update.
- **The trigger depends on `cfg80211` loading early.** If a later firmware stops loading the Wi-Fi module at boot, only the udev backup is left; the canary can re-measure.
- **Switched off, not hidden.** The internal storage's safety rests on the switch-off service running first and powering off on failure.
- **Installing needs developer mode and SSH once per MPC.** A musician cannot install the hook alone yet.
- Not recorded yet: what the screen shows between the MPC logo and MPFree's startup screen.

Next steps are kept in [the record](stick-autoboot.md#next-steps).
