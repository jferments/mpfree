# MPC One Linux and Boot Architecture
## Comprehensive engineering reference for MPFree — independent, owner-controlled operating system

**Revision:** 2026-11-13 · **Hardware:** Original Akai MPC One, Gen 1, Rockchip RK3288 · **Observed installed system:** MPC OS 3.6.0 (with documented developer modifications) · **Successfully tested temporary system:** 3.10.0 · **Assessed offline:** 3.10.1

> **Scope and evidence policy.** This is a developer-facing architectural specification and implementation guide, not a claim that every theoretical recovery capability has been exercised on the actual MPC One. It incorporates the November 8–13, 2026 hardware work, verified owner-held storage backup, runtime trials, and the 3.10.1 firmware comparison. Vendor images, individual device identifiers, SSH credentials and proprietary sample content are not included. Findings apply to the reference original MPC One and must not be silently generalized to One+, Live, X, Force or Gen 2.

**Central conclusion:** We have already booted another Linux kernel/userspace on the MPC One **three times** using `kexec` and removable storage, with no changes to the factory boot-critical eMMC regions and reliable return to factory Linux after a power cycle. We also know the structure of the stock BootROM → first-stage loader → U-Boot → Linux path, and the pre-Linux Fastboot implementation. What we have **not** demonstrated is an independent *cold boot* of an owner-built OS without first running factory Linux, or an independent ROM-based restoration method for a damaged first-stage loader. Those are separate, explicit milestones.

### Evidence vocabulary

| Classification | Interpretation |
|---|---|
| **Observed on device** | Actually measured or executed on this MPC One, including repeatable hardware behavior. |
| **Verified in image** | Proven from the existing bootloader/firmware bytes, environment, image headers or scripts; not automatically a successful real-world operation. |
| **Public platform behavior** | Documented RK3288, U-Boot or inMusic/Akai capability, subject to device-specific validation. |
| **Inferred** | Plausible interpretation of evidence; not a completed hardware test. |
| **Proposed** | Design option or planned experiment. |
| **Unknown** | Insufficient evidence; installer must not assume it works. |

---

## 1. Hardware and factory system identity

| Component | Reference-unit details | Status |
|---|---|---|
| SoC | Rockchip RK3288, four ARMv7 Cortex-A17-class cores, NEON, VFPv4 | Observed |
| RAM | Approximately 2 GiB physical, no swap observed | Observed |
| Linux | `6.6.95-az01-2025-06-27-rt55`, SMP PREEMPT_RT | Observed on 3.6.0 |
| Distribution | inMusic `az0x` on Yocto/OpenEmbedded Scarthgap | Observed |
| Userspace ABI | 32-bit little-endian ARM EABI5 hard-float; glibc 2.39; `/lib/ld-linux-armhf.so.3` | Observed |
| eMMC | DG4008, manufacturer ID `0x45`, 7,820,083,200-byte user area (~7.28 GiB) | Observed |
| Filesystems | GPT, ext4 root/content/data, persistent overlayfs and RAM-backed tmpfs | Observed |
| Audio | ALSA `ACVA` capture/playback; separate MPC MIDI card | Observed |
| Display/GPU | DSI display 800×1280, DRM/KMS, framebuffer, Panfrost/Mali GPU | Observed |
| Input | Ilitek ILI2117 touchscreen, GPIO keys, evdev | Observed |
| Ethernet | GMAC at `0xff290000`, RTL8211E PHY, 1-Gb/s link tested | Observed |
| USB | Host controller `ff540000`; DWC2 OTG/peripheral controller `ff580000` | Observed |
| Main application | `/usr/bin/MPC` launched by `/usr/bin/az01-launch-MPC` and `acvs.service` | Observed |
| OS handoff | `kexec` supported, working physical test with 3.10.0 | Observed |

The MPC One is an ordinary embedded Linux computer with realtime-oriented audio and a dedicated hardware-control stack. The factory music application is not the bootloader. It is a service launched after the kernel, mounts and systemd are running. Accordingly, developing a completely independent music-production userspace does **not** require altering the boot ROM or proprietary application, although controlling the power-on boot path eventually will.

## 2. The complete normal boot chain

```text
RK3288 silicon reset
    |
    v
[0] RK3288 BootROM: silicon loader, media selection, optional ROM USB mode
    |
    v
[1] eMMC user area at LBA 64: Rockchip RK32 ID block
    + 32-KiB SRAM loader from LBA 68, executing near 0xFF704000
    |
    v
[2] eMMC p3 at LBA 256: legacy-uImage U-Boot proper
    + p2: U-Boot environment
    |                         \
    | normal boot              \ recovery request
    v                           v
[3] U-Boot distro boot scan     U-Boot Fastboot USB Update Mode
    boot_targets=mmc0 mmc1     (before Linux / rootfs is mounted)
    /boot/boot.scr.uimg
    |
    v
[4] p6:/boot/zImage + board .dtb -> bootz
    |
    v
[5] Linux 6.6 PREEMPT_RT + systemd
    / from p6 (read-only); /data from p8 (read-write)
    /etc and /var as persistent overlays on p8
    |
    v
[6] acvs.service -> stock MPC app, audio/UI/input and device services
```

**Confidence boundary:** The partition layout, software components, kernel boot files and environment are real. Selected early-loader code paths and recovery logic are verified in images, but the complete power-on path has not been instrumented with independent serial tracing. A diagram of control flow is not proof that arbitrary replacement images are accepted.

### 2.1 RK3288 BootROM

The BootROM is immutable code in the SoC. Public RK3288 documentation describes its search for recognized Rockchip boot media images and a USB ROM recovery protocol. This stage executes **before** eMMC U-Boot, Linux, services or developer SSH. It must not be confused with Akai's U-Boot-based “Update Mode.”

On this MPC One, a valid Rockchip ID block resides in the eMMC user area at **sector 64**, which explains the normal eMMC-first path. BootROM recovery could ultimately provide owner-controlled recovery even when the installed U-Boot is missing, but **genuine USB ROM enumeration, owner-built SRAM code execution and independent eMMC restoration have not been demonstrated on the reference device**.

BootROM media search order is not the same thing as U-Boot's `boot_targets` setting. An SD card containing Linux files does not necessarily affect the silicon ROM's choice when a valid eMMC ID block is already available.

### 2.2 Rockchip ID block and first-stage SRAM loader

**Verified in image:**

- `p1`, GPT name `uboot-spl`, begins at sector 64 and spans 64 KiB.
- The decoded Rockchip header has signature `0x0FF0AA55` and field `disable_rc4=1`. The header format has a historical Rockchip transformation; the executable payload is unencrypted. This says nothing by itself about firmware integrity policy.
- The executable first stage occupies approximately 32 KiB from sector 68, and is loaded into internal SRAM near `0xFF704000`.
- Header `init_size` equals 64 sectors (32 KiB); `init_boot_size` equals 1,088 sectors (544 KiB). The latter is consistent with Rockchip image-packaging allowances and does **not** mean that 544 KiB is contained in p1.
- A startup sequence is identified near `0xFF704020 → 0xFF704064 → 0xFF7040B4 → 0xFF705208`.
- A boot-selection function is associated with `0xFF705350`; an MMC loader near `0xFF7054A4` reads sector `0x100` (256), recognizes legacy uImage magic `0x27051956` and parses second-stage image metadata.
- An image-header-derived handoff slot was identified near `0xFF70A250`. Exact final branch selection, fallback and board-specific behavior remain unresolved.

These numeric code addresses describe **one captured loader**, not a stable programming interface for future firmware versions. Early loader replacement is unnecessary for the next MPFree stage and remains outside the safe initial installer scope.

### 2.3 U-Boot proper and its persistent environment

**Verified in image:** `p3` at LBA 256 holds a 384-KiB second-stage image beginning with legacy uImage magic `0x27051956`. `p2` at LBA 192 holds an 8-KiB U-Boot environment, readable by `fw_printenv` using `/etc/fw_env.config`. Board-specific variables such as `fdtfile` and `dorecovery` are set or interpreted during boot.

Representative environment, with machine-specific values removed:

```text
bootcmd=run az01_bootcmd
az01_bootcmd=if test x(dorecovery) = x1; then run az01_recovercmd; fi;
             run distro_bootcmd;
             echo 'Boot failed, resetting to bootloader...';
             reset_bootloader
az01_recovercmd=echo 'Starting fastboot...'; fastboot 0
boot_targets=mmc0 mmc1
boot_prefixes=/ /boot/
boot_scripts=boot.scr.uimg boot.scr
bootdelay=0
stdin=serial,gpio-keys
stdout=serial
fastboot.partition-type:rootfs=ext4
fastboot.partition-type:content=ext4
fastboot.partition-type:data=ext4
```

The first line above is a **readable schematic** of the environment condition; the original literal uses U-Boot variable expansion for `dorecovery`. U-Boot has an 86-command registry containing `mmc`, `load`, `ext4load`, `fatload`, `bootz`, `source`, `run`, `setenv` and `saveenv`. Its distro scan can look for boot scripts and, where configured, extlinux entries.

Linux's FDT maps `ff0f0000` to internal eMMC and `ff0c0000` to removable SD. The **U-Boot** numbering of `mmc0`/`mmc1` has not been independently validated against these Linux aliases. Even if they match, the first functional boot target normally wins; inserting an SD card does not prove it will preempt a healthy eMMC installation.

### 2.4 Boot script, kernel and board description

There is **no separate kernel GPT partition**. The stock `p6` rootfs contains:

- `/boot/zImage-6.6.95-az01-2025-06-27-rt55` (approximately 5.6 MiB);
- `/boot/boot.scr.uimg` (535 bytes);
- board trees including `rk3288-az01-acva.dtb` and other `az01`/`az05` variants.

The script loads `zImage` and `fdtfile` from its selected partition, optionally applies `rk3288-az01b.dtbo` for that board variant, appends `fsck.repair=yes` and calls `bootz`. This reference unit needs `rk3288-az01-acva.dtb`, not `-acva-c` (which differs in CPU frequency entries). U-Boot can modify the live FDT at startup; a packaged DTB and the FDT handed to Linux are not necessarily byte-identical.

Observed kernel command-line concepts:

```text
root=PARTUUID=<factory-rootfs-uuid> rootwait ro
rfkill.default_state=0 fsck.repair=yes isolcpus=2-3
```

A custom boot path must supply the right root argument, memory reservations, IRQ routing, clocks, PHY reset timing, display parameters and supported board profile.

### 2.5 Systemd and factory app layer

Once Linux mounts its filesystems, systemd activates board facilities and `acvs.service`. The service launches the MPC executable and has automatic restart behavior. The factory process normally owns ALSA audio, MIDI, DRM/display, touchscreen and GPIO-key input. A custom application should never assume it can use these devices concurrently with the factory app. Safe runtime handoff requires an orderly service stop, a supervisor, a health check and a tested restart path; killing one PID does not satisfy this.

## 3. eMMC organization and block-level access

Every LBA below is **relative to the 512-byte-sector user area of the reference eMMC**.

| Linux partition | GPT label | Start LBA | Sectors | Size | Role |
|---|---|---:|---:|---:|---|
| `mmcblk0p1` | `uboot-spl` | 64 | 128 | 64 KiB | Rockchip ID block + SRAM first stage |
| `mmcblk0p2` | `env` | 192 | 16 | 8 KiB | Persistent U-Boot environment |
| `mmcblk0p3` | `uboot` | 256 | 768 | 384 KiB | U-Boot proper |
| `mmcblk0p4` | `splash` | 1,024 | 8,192 | 4 MiB | Boot imagery |
| `mmcblk0p5` | `recoverysplash` | 9,216 | 8,192 | 4 MiB | Recovery imagery; not proof of recovery Linux |
| `mmcblk0p6` | `rootfs` | 17,408 | 1,024,000 | 500 MiB | ext4 factory Linux and `/boot` |
| `mmcblk0p7` | `content` | 1,041,408 | 8,388,608 | 4 GiB | Content/assets |
| `mmcblk0p8` | `data` | 9,430,016 | 5,843,551 | ~2.79 GiB | User data and persistent system overlays |

**Observed physical user area:** 7,820,083,200 bytes (~7.28 GiB). GPT metadata, alignment regions and backup GPT are separate from the named ext4 filesystems. Whole-disk image size is not identical to the sum of partition contents. Device-specific GUIDs, serial numbers and MAC addresses must not be included in public examples.

### 3.1 Hardware eMMC boot areas, EXT_CSD and RPMB

Apart from `/dev/mmcblk0`, Linux exposes `/dev/mmcblk0boot0` and `/dev/mmcblk0boot1`, each 4 MiB. Both contained **all zeroes** in the reference read-only acquisition; `force_ro=1` was observed. This is consistent with normal boot code being held in the user-area ID block. It does not entitle an installer to assume every device or firmware release has an identical EXT_CSD boot configuration.

EXT_CSD may control eMMC hardware boot selection and protection. RPMB is a separate authenticated eMMC area whose state is **not** covered by an ordinary user-area `dd` copy. RPMB provisioning, hardware boot enable settings, write protection and any eFuse/OTP state are **not** part of the permitted early installer write surface.

### 3.2 Backup evidence and its limits

An owner-held full read-only backup captured `mmcblk0` and both boot areas. Device-side and copy-side SHA-256 measurements agree for GPT/MBR and boot-critical `p1`–`p6`; boot0/boot1 also matched. The p1, p3–p6 baseline measurements were repeated around the temporary boot tests. This is strong evidence that a complete **copy exists**.

It is **not evidence that the backup can be restored after boot failure**. Writable `p7`/`p8` can change while the copy is being taken, so their data represents a potentially non-transactional snapshot. eMMC configuration and other one-time state are distinct from disk image bytes. The recovery design must be independently qualified.

### 3.3 Linux device access and persistent filesystem structure

Observed mount layout on firmware 3.6.0:

| Source | Mount | Properties |
|---|---|---|
| `/dev/mmcblk0p6` | `/` | ext4, read-only, no journal |
| `/dev/mmcblk0p8` | `/data` | ext4, read-write |
| same `p8` | `/media/az01-internal` | second mount of **same** data partition |
| `/dev/mmcblk0p7` | `/media/acvs-content` | factory content, writable |
| overlay | `/etc` | persistent upperdir `/data/system/etc/overlay` |
| overlay | `/var` | persistent upperdir `/data/system/var/overlay` |
| tmpfs | `/run`, `/tmp` and several runtime locations | volatile |
| USB storage | `/media/<name>` | commonly `nosuid,nodev,noexec` |

**Critical installer rule:** Read-only `/` does **not** make `/etc` or `/var` read-only. Changes to those paths often survive reboot because their overlay upperdirs are on `p8`. Nor is `/media/az01-internal` independent storage from `/data`. Writing `p6` rootfs or the 8-KiB `p2` environment is qualitatively different from writing isolated RAM-backed `/run` files.

The eMMC controller corresponds to the Rockchip MMC node at `ff0f0000`. On a specially prepared temporary kernel, marking this node `disabled` prevents the MMC driver from binding and therefore prevents user programs from opening its normal block devices. This was a key part of the proven temporary boot arrangement.

### 3.4 Hash and change monitoring

The original repository's [boot map](boot-map.md) records specific p1/p3–p6, GPT and boot0/boot1 checksums against reference state `MOD-001+MOD-002`. The test programs also inspect Linux block-device sectors-written counters. Use both hashes and write counters: a zero counter delta alone cannot prove absence of every possible storage alteration, and a single checksum may not cover data or eMMC configuration. Do not publish the full disk image or owner credentials.

## 4. Linux permissions, execution model and ABI

### 4.1 Root userspace is not the boot ROM

Earlier development sessions obtained key-authenticated root SSH through an opt-in USB `autorun.sh` entry, then created a persistent `mpfree-sshd.service`. The reference device modifications are explicitly recorded as `MOD-001` (rootfs service) and `MOD-002` (keys/config on `/data`); its p6 is therefore **modified firmware 3.6.0**, not an untouched official rootfs. Future official firmware installation may replace the service while keeping data or overlay material.

Root shell access is enough to inspect Linux, control services and potentially launch a different kernel. It does **not** itself confer control of the silicon ROM's decisions, prove owner-supplied images are accepted by the installed bootloader, or ensure recovery from corrupted p1/p3.

### 4.2 The verified `kexec` facility

**Observed:** Factory kernel options include `CONFIG_KEXEC_CORE=y`, `CONFIG_KEXEC=y`, `CONFIG_KEXEC_FILE=y` and `CONFIG_IKCONFIG_PROC=y`; `/usr/sbin/kexec` exists. The manufacturer itself uses `kexec` to start its RAM updater for USB/network firmware update paths. More important: the reference MPC booted isolated 3.10.0 Linux from removable media via `kexec` **three times**.

`kexec` prepares a replacement kernel and associated data in RAM and jumps to it from running Linux, avoiding the normal cold-start BootROM/SPL/U-Boot progression for that second kernel. It creates real kernel/userspace independence **after the handoff**, but **requires the installed Linux system to start first**. It is therefore our strongest tested *reversible development boot*, not a complete direct-power-on solution.

### 4.3 Runtime ABI, device rights and RT behavior

The system's primary ABI is ELF32 ARM EABI5 hard-float with `/lib/ld-linux-armhf.so.3` and glibc 2.39. For Rust use a board-matched `armv7-unknown-linux-gnueabihf` build; check ELF `PT_INTERP`, `DT_NEEDED`, versioned `GLIBC_*` requirements and ARM feature assumptions before deployment. The factory's PREEMPT_RT kernel and ALSA/DRM/input interfaces can support an open-source realtime DAW. Rust-first MPFree components have already run successfully.

Service process permissions, exclusive device ownership, Linux kernel capabilities and loader-specific image acceptance are separate questions. A functional drum sequencer running in the factory Linux environment is not proof of full device takeover or cold boot.

### 4.4 Load-time integrity facilities

The 3.10.0 build had `CONFIG_SECURITY_LOADPIN_ENFORCE=y` recorded; exact behavior for modules or firmware sourced from removable media was not fully settled. The 3.10.1 comparison saw no new generic kernel Lockdown, mandatory module-signature or IMA enforcement in the compared kernel config. Yet another component, `az01-signed-fs`, changed substantially, and its runtime consequences are **unknown**. Developers must not equate the absence of one generic kernel setting with unrestricted acceptance of all images at all boot stages.
