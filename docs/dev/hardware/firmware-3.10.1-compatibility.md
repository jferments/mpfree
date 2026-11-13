# MPC OS 3.10.1 — Developer Access and Boot Compatibility

**As of November 13, 2026** · **Scope:** Original MPC One Gen 1 · **Evidence:** Official feature descriptions plus *offline* 3.10.1 firmware comparison. The reference device has **not** been upgraded for this assessment.

For full details read [the canonical architecture](mpc-one-boot-architecture.md#10-mpc-os-3101-official-developer-facilities-and-boot-system-compatibility).

## Official developer interfaces

MPC OS 3.10.1 introduces optional **Developer Panel**, built-in Linux terminal, **SSH activation**, resource/log monitoring and an official ARM32/ARM64 [MPC Plugin SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk). The SDK installs native instruments/effects into the factory MPC host; it does **not** mean a complete alternative Linux OS is officially installable. Remote screen/audio, external plugin support, USB mouse and screenshots are additional optional enhancements. Enable external plugins under **Preferences → Enhancements**, according to the firmware's interface.

The Developer Panel means an owner need not use the earlier USB `autorun.sh` technique solely to obtain an interactive Linux shell. A high-access SSH service should run only on a controlled network, with credentials/authentication verified. Exact port, default login, persistence and privilege behavior on this physical MPC have not yet been established.

## Offline comparison findings

| Component | Result |
|---|---|
| Kernel image, board DTBs, boot script | Unchanged in examined comparison |
| Kernel `CONFIG_KEXEC_CORE`, `CONFIG_KEXEC`, `CONFIG_KEXEC_FILE` | Enabled |
| Generic Lockdown/module-signature/IMA config | No new enforcement detected in examined kernel |
| Updater and `libaz01-prog` | Identical in compared files |
| Update RAMFS | Difference limited to `/etc/timestamp` and `/etc/version` |
| Existing SSH/autorun components | No change found in compared areas |
| Bootloader signature/LoadPin helper code/config | No differences found in compared artifacts |
| `az01-signed-fs` | **Changed materially; exact runtime consequences unresolved** |

These are **image-level findings**, not hardware validation after a 3.10.1 update. They do not establish installed bootloader/eMMC state, Fastboot acceptance of owner images, physical kexec execution, signed-filesystem behavior, rollback or independent recovery.

## Update gate

Retain verified off-device backups; complete analysis of the changed integrity component; qualify SSH/authentication and per-partition changes on an appropriate device; and preserve a proven factory return path. Do not treat the existence of official SSH as proof of independent OS boot.

**Sources:** [Akai firmware](https://www.akaipro.com/downloads-and-support/downloads/firmware/mpc/), [release notes](https://www.akaipro.com/mpc-release-notes/), [inMusic SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk).
