# MPC OS 3.10.1 — Developer Access and Boot Compatibility

**As of November 15, 2026** · **Scope:** Original MPC One Gen 1 · **Evidence:** Official feature descriptions plus *offline* 3.10.1 firmware comparison and the USB-stick tests. The assessment below was made before the install. **The owner installed 3.10.1 on the reference device on 2026-11-15**, with developer mode on. The [qualification runbook](firmware-3.10.1-qualification.md) checks main against the installed system: logging in, a fresh backup, the kexec test boot and the way back.

For full details read [the canonical architecture](mpc-one-boot-architecture.md#10-mpc-os-3101-official-developer-facilities-and-boot-system-compatibility).

## Official developer interfaces

MPC OS 3.10.1 introduces optional experimental enhancements, switched on under **Menu → Preferences → Enhancements**: a **developer panel**, external plugin support with an official ARM32/ARM64 [MPC Plugin SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk), remote screen and audio, USB mouse and screenshots. The SDK installs native instruments/effects into the factory MPC host; it does **not** mean a complete alternative Linux OS is officially installable.

According to Akai's [3.10.1 release notes](https://cdn.inmusicbrands.com/Software/MPC/MOPN4PCOPEN/MPC%203.10.1%20-%20Release%20Notes.pdf), developer mode is entered by holding SHIFT + MENU and choosing Developer from the mode menu. The panel has an on-screen terminal, `dmesg`, `journalctl`, services, a resource monitor, voice statistics and plugin management. Its shell has "full access to this product's Linux system, with the same rights as the product itself"; turning it on also starts the SSH server, and turning it off stops both. It adds a **script runner that runs `autorun.sh` on any drive while in developer mode**. The notes name no port, login or password, and say nothing about booting another kernel.

The 3.10.1 image itself answers most of that (offline survey, 2026-11-13; details in the [stick test runbook](firmware-3.10.1-stick-test.md#phase-1-results-2026-11-13)). Akai's app runs `/usr/lib/az0x/user-access/enable.sh`, which copies a skeleton into `/data`, mounts an overlay on `/etc` and reloads sshd. The result is root SSH on port 22 with **no authentication** (`AuthenticationMethods none`, an empty root password, `PermitEmptyPasswords yes`), as a community FAQ had said ([Open MPC](https://open-mpc.github.io/faq.html)). Akai's own lock-down is to empty `/data/ssh/sshd_config.d/allow-access.conf` and put a key in `/data/ssh/authorized_keys`. Turning developer mode off deletes `/data/root` and `/data/ssh` and reboots. The script runner is part of Akai's app and asks on the screen before running a drive's `autorun.sh`.

On the 3.10.1 test stick (2026-11-14, [phase 3's first run](firmware-3.10.1-stick-test.md#phase-3-first-run-2026-11-14-clone-dcce005)), developer mode came on from Akai's app as the files say. Akai's SSH server was not running before it: `enable.sh`'s reload failed, and `sshd.service` then started and listened on port 22. Turning it on also tried to start Akai's web server (`az0x-webserver.service`, masked on the stick). 3.10.1's firewall rules files are empty, so its firewall units load no rules from the image.

The developer mode means an owner need not use the earlier USB `autorun.sh` technique or a rootfs change (MOD-001) solely to obtain a shell. Because it opens without authentication, it should run only on a controlled network and be locked to a key at once. Whether it stays on across reboots, and how it behaves on this physical MPC, have not been established: the overlay on `/etc` lives only in RAM, so that depends on the app (an inference from the files).

## Offline comparison findings

| Component | Result |
|---|---|
| Kernel image, board DTBs, boot script | Unchanged in examined comparison |
| Kernel `CONFIG_KEXEC_CORE`, `CONFIG_KEXEC`, `CONFIG_KEXEC_FILE` | Enabled |
| Generic Lockdown/module-signature/IMA config | No new enforcement detected in examined kernel |
| Updater and `libaz01-prog` | Identical in compared files |
| Update RAMFS | Difference limited to `/etc/timestamp` and `/etc/version` |
| Existing SSH/autorun components | No change found in compared areas; shipped sshd is root by key only, root password locked |
| Developer mode (`/usr/lib/az0x/user-access`) | New: opens root SSH with no authentication when switched on (above) |
| Peripheral firmware (touch, pad controller DFU and SysEx, Wi-Fi/Bluetooth) | 35 of 38 files identical to 3.6.0; the 3 that differ (`regulatory.db`, its signature, `SME0/Firmware/update.sh`) are not flashed to any chip |
| Bootloader signature/LoadPin helper code/config | No differences found in compared artifacts |
| `az01-signed-fs` | **Changed materially; exact runtime consequences unresolved** |

These are **image-level findings**, not hardware validation after a 3.10.1 update. They do not establish installed bootloader/eMMC state, Fastboot acceptance of owner images, physical kexec execution, signed-filesystem behavior, rollback or independent recovery.

## Update gate

Retain verified off-device backups; complete analysis of the changed integrity component; qualify SSH/authentication and per-partition changes on an appropriate device; and preserve a proven factory return path. Do not treat the existence of official SSH as proof of independent OS boot. The [stick test runbook](firmware-3.10.1-stick-test.md) covers the checks that can be made before installing. The owner installed 3.10.1 on 2026-11-15 after those checks. The [qualification runbook](firmware-3.10.1-qualification.md) covers the checks after it, starting with a fresh backup of the installed system.

**Sources:** [Akai firmware](https://www.akaipro.com/downloads-and-support/downloads/firmware/mpc/), [release notes](https://www.akaipro.com/mpc-release-notes/), [inMusic SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk).
