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


## 5. USB controller architecture and boot-mode distinctions

The word “USB” covers **at least five different functions** on this board. A connector's physical presence cannot by itself establish which stage controls it.

| Function | Controller/code | Available during | Use | Dependence |
|---|---|---|---|---|
| Normal USB-A host | Linux and `usb@ff540000` host-side logic | Running factory/custom Linux | USB disks, MIDI, peripherals, autorun stick | Requires Linux |
| Linux USB device/gadget | DWC2 peripheral `usb@ff580000` | Running compatible Linux | USB functions configured through Linux | Requires Linux |
| Akai Update Mode | U-Boot `fastboot 0` | Pre-Linux, after p1/p3 load | Desktop firmware update, queries, image transfers | Requires stock first stages |
| RockUSB-compatible loader mode | Loader in RAM or U-Boot | Loader-specific | Rockchip-style flash/loader transfer | Depends on actual loader and entry path |
| RK3288 silicon USB ROM mode | Immutable SoC BootROM | Before RAM loader/installed U-Boot | Initial SRAM code load and potential recovery | No factory Linux required; entry not yet proven |

**Observed:** Linux's live device tree identifies `usb@ff580000` as `rockchip,rk3288-usb` / `snps,dwc2` with `dr_mode=peripheral`. A corresponding UDC appears at `/sys/class/udc/ff580000.usb`. Another DWC2 controller at `ff540000` is in host mode. A board-specific `usb-otg-mux` advertises `otg`, `host` and `peripheral` states. The complete physical cabling, regulator switching and safe runtime mux behavior must be qualified separately.

**Inferred:** The rear USB-B connector used by the desktop updater is linked to the `ff580000` peripheral path; it is therefore a strong candidate for silicon ROM USB access, if the SoC can be placed in that mode. USB-B Fastboot enumeration alone would not prove this ROM path is active.

### 5.1 USB removable filesystems and executable files

Factory automount commonly uses `noexec` for FAT/exFAT USB media. The original `autorun.sh` mechanism works because the vendor script-runner interprets a shell file; a compiled ARM executable is **not** thereby guaranteed to execute directly from the mounted USB partition. Validated approaches include copying a checked binary into volatile executable `/run` storage, mounting a suitably prepared external Linux filesystem (subject to its mount flags), or starting an isolated USB root with `kexec`. Avoid modifying rootfs merely to defeat a removable-media mount restriction.

### 5.2 U-Boot Fastboot and recovery path

**Verified in image:** The board has a Fastboot-oriented recovery selection. SARADC channel 1 at a threshold near `≤99` affects a shared state flag at approximately `0x001462FC`. U-Boot startup interprets that state through `dorecovery` and `az01_recovercmd`, entering `fastboot 0` before the normal Linux rootfs is read. The manufacturer documents a power-on Update Mode button combination (FULL LEVEL + BROWSE + PAD BANK C/G), but the actual gesture and subsequent recovery steps have **not** been end-to-end rehearsed on this reference device.

U-Boot registers the Fastboot command family `getvar`, `download`, `boot`, `continue`, `flash`, `erase`, `oem` and `reboot`. The offline program study found RAM receive base `0x00800800`, length/cursor globals near `0x0014D684`/`0x0014D680`, and an advertised download limit `0x70000000`. **That advertised number is not a proven safe physical RAM budget.** The `boot` dispatch path reaches a `bootm`-related image execution function, but accepted image format, real execution, FDT handoff and memory safety remain unproven.

The U-Boot `oem` command family includes a board-specific authorization workflow associated with write/erase/download handling. Those functions belong to the installed firmware's maintenance interface; this architecture does not rely on using them. What matters for planning is the **capability boundary**: access to Fastboot does not mean every command is safe or currently permitted.

The early-stage `reset_bootloader` logic sets a Fastboot-related Rockchip reboot register value `0x5242C309` at `0xFF730094`. **This does not demonstrate any usable software transition into silicon-ROM USB mode.** Other register patterns that may apply to other boards and loaders must not be assumed to work on the MPC One.

### 5.3 Updater USB backends and semantics

Offline 3.10-era analysis isolated three transport implementations, including Fastboot request family `FF/42/03`, another `FF/61/7A` backend, and a RockUSB-compatible interface. Their streaming, write and commit pathways differ. Their static state transitions and synthetic host models do **not** demonstrate physical USB enumeration, acceptance of a whole image, safe writes or recovery on the MPC One. A portable installer must select only a **physically qualified** backend for the observed device/firmware, and should not treat transports as interchangeable.

### 5.4 What USB means for truly independent boot

Akai's U-Boot Fastboot mode is a **pre-Linux** interface and potentially valuable for nonpersistent RAM boot and factory rootfs recovery. It **still requires intact p1/p3**. By contrast, genuine silicon-ROM mode could potentially start an independently developed SRAM loader with no factory boot code, but entering that mode and executing code on the physical MPC remain open.

It is therefore inaccurate to describe the current USB-B Update Mode as a guaranteed way to recover **any** storage failure. Its strongest plausible role is recovery or update of p6/p7/p8 while the original first-stage and U-Boot are functional.

## 6. A demonstrated fully reversible Linux boot: 3.10.0 via kexec

This is the most important hardware result to carry into a new engineering effort. It happened on the actual MPC One, not just in a virtual machine.

### 6.1 Boot arrangement

```text
Power-on → factory RK3288 loader → factory U-Boot → factory Linux 3.6.0
   |
   | owner initiates prepared kexec from verified transient stage
   v
3.10.0 zImage + modified board FDT + USB-resident rootfs
   |
   | internal eMMC and SD controllers disabled in new FDT
   | /data is tmpfs
   | update/formatting services are masked
   | second kexec is disabled
   v
3.10.0 independent runtime: services, display, Ethernet and SSH
   |
   | physically power cycle
   v
Unchanged factory U-Boot and 3.6.0 rootfs from eMMC
```

The test image explicitly marked both Rockchip MMC controllers (eMMC `ff0f0000` and SD `ff0c0000`) `disabled`, preventing Linux from creating those corresponding block devices. It also substituted RAM for `/data`, prevented further `kexec`, masked manufacturer updater/formatter functions, and replaced dangerous maintenance utilities with inert controls. The replacement system had no normal MMC devices available to its programs. A writable external log area did remain available. **Any process that intentionally changes another supported physical peripheral would need a separate safety review; this particular storage result is not a universal promise of zero side effects.**

### 6.2 Trial history

| Actual run | Result | Lesson |
|---|---|---|
| **First, 2026-11-10** | 3.10.0 userspace started ~7.7 s after new kernel start; SSH service listened but Ethernet PHY detection failed; on-screen output not yet available. Boot storage unchanged. | Need correct live board FDT and explicit PHY initialization. |
| **Second, 2026-11-10** | Live screen status/log visible; PHY recognized as RTL8211E; initial GMAC DMA setup timed out. Boot storage unchanged. | Display is viable; warm-reset PHY/clock state affects network. |
| **Third, 2026-11-10** | Initial network setup failed, but a later `ip link set eth0 up` succeeded; link 1 Gb/s, DHCP and authenticated SSH worked; boot storage unchanged. | One retry after PHY reinitialization is a proven workaround. |

The first kernel had trouble identifying the Ethernet PHY immediately after the warm handoff. The test FDT added `ethernet-phy-id001c.c915` and preserved the Realtek RTL8211E reset behavior on GPIO4 B0 (10 ms assertion and 30 ms deassertion). On the third run, the network retry at about 20.6 seconds succeeded, gigabit link arrived around 23.1 seconds, DHCP around 28 seconds and workstation SSH by ~32.9 seconds. The low-level reason for the first DMA timeout remains **unresolved**, despite comparison of clock and reset state before/after the test. Do not elevate EEE or clock-settling hypotheses into facts.

### 6.3 Integrity observation

Partition hashes and write counters for the boot-critical regions were checked before and after temporary boots against the saved baseline. `p1` through `p6` and boot0/boot1 retained their reference contents. The 3.10.0 test kernel reported no MMC block device. Power cycling brought back the factory 3.6.0 boot. This demonstrates a **real no-eMMC-write Linux test environment** and motivates the same design for early MPFree kernel builds.

### 6.4 What this proves—and what it does not

**Proven:** Another Linux kernel can run; another isolated userspace can start from USB; local screen and, after retry, network/SSH operate; factory boot remains available on reboot; tested boot-critical eMMC regions were unchanged.

**Not proven:** Kernel acceptance by Fastboot or silicon ROM; U-Boot booting owner files directly from SD; an independent MPFree distro power-on boot; recovery from written/invalid p1 or p3; direct physical 3.10.1 kexec test; ability to freely modify eMMC while preserving a reliable restore option.

**Recommended use:** Treat `kexec` as the lowest-risk *qualified means of running a wholly different kernel*, not as an already independent cold boot.

## 7. What is the lowest-level reliable boot access today?

A precise answer requires distinguishing **three different senses of “lowest-level”**:

1. **Actually exercised to launch owner-controlled Linux on this hardware:** Linux root plus `kexec`. It starts after the factory Linux boot but has a demonstrated reversible outcome when storage controllers are disabled.
2. **Earliest installed pre-Linux software with meaningful boot functions:** Factory U-Boot, with MMC/file loaders, `bootz`, recovery selection and Fastboot. Its code path exists without p6, but arbitrary owner kernel RAM boot and fallback have **not** been validated on real hardware.
3. **Earliest silicon-independent recovery mechanism:** RK3288 BootROM USB mode. It is a documented platform facility, but genuine device entry, execution of an independently built first stage and recoverable eMMC service remain **unknown** for this board.

**There is currently no proven, fully autonomous MPFree cold-boot selection that meets the project's strict stock-preservation standard.** U-Boot's existing features are enough to formulate credible tests; they are not permission to change the boot-critical environment today.

## 8. Candidate custom-OS boot paths, ranked by maturity

| Path | Starts without factory Linux? | eMMC boot-critical writes? | State |
|---|---|---|---|
| A. New MPFree userspace with factory kernel | No | None required for session-only use | Viable integration path |
| B. Custom MPFree Linux by `kexec` from USB | No | None in proven isolated design | **3.10.0 form demonstrated** |
| C. U-Boot boot from SD/USB-compatible storage | Potentially yes | None if selection already supported; otherwise p2 write | **Unproven selection** |
| D. U-Boot Fastboot RAM `boot` | Potentially yes | Not inherently, for a RAM-only command | **Unproven execution format** |
| E. Full independent SRAM/DDR loader from silicon ROM | Yes in principle | None for RAM-first tests | **ROM entry/loader unproven** |
| F. Factory p6 replacement with MPFree root | Yes, if existing U-Boot accepts it | **Yes: destructive replacement** | Credible, **not yet safe** |

### 8.1 A — independent userspace on factory Linux

Build the Rust-first music environment, using C/C++ libraries only as needed for hardware integration. Start it from a reversible developer-controlled session, or from a purpose-built test rootfs where `acvs.service` is disabled. Stop/release the factory audio/UI stack cleanly and restore it on failure. This exploits the already working RT kernel, ALSA, DRM, MIDI, display and other drivers. It does **not** remove dependence on factory Linux or on its initial boot.

### 8.2 B — independent Linux via kexec

Build and stage an MPFree kernel, correct FDT, initramfs and rootfs on the workstation/removable media. Use the observed 3.10.0 test technique: exact board identity, MMC disabled, RAM `/data`, update services masked, no additional kexec and observable boot logs. Add a watchdog and explicit power-cycle fallback. Once the OS has run without any internal MMC access, qualify audio/UI/network, then only consider controlled writable storage later. This is the **recommended immediate development path**.

### 8.3 C — U-Boot booting an SD image

The installed U-Boot contains `mmc`, `ext4load`, `fatload`, `source`, `bootz` and distro boot scanning. A self-contained SD image with compatible kernel/FDT/boot script and root filesystem is therefore a plausible cold-boot target. **Still required:** empirically map U-Boot MMC indexes to sockets, discover how an alternative target is chosen with healthy p6 present, test board reset state, understand any extlinux/boot script precedence, and prove that removal of the SD returns to factory boot. The current `boot_targets` order is **not** proof of SD precedence.

Changing `p2` with `saveenv` is a boot-critical persistent change; it is **not** a suitable first test just because the partition is small.

### 8.4 D — U-Boot Fastboot booting a RAM image

The existing Fastboot `download`/`boot` callback tree offers a potential nonpersistent pre-Linux execution interface. To qualify it, determine the **actual** accepted container (e.g., legacy image vs ARM `zImage`), entrypoint and FDT data handling, validated RAM bounds and recovery response to invalid images. The currently advertised download maximum is not a safe test payload target. There is no physical demonstration of a working arbitrary ARM kernel RAM boot through this command. Test only in a strictly no-flash context.

### 8.5 E — independent silicon-ROM first-stage loader

This is the long-term route to independence from the installed loader and to recovery after early-stage failure. A complete proof requires real ROM USB entry, a source-built SRAM stage accepted and executed on this board, DRAM initialization if needed, correct USB/serial observability, independent access to eMMC blocks and checked recovery tooling. Public RK3288 documentation and other boards are references, not substitutes for this proof.

Do not erase the existing loader to induce ROM recovery. Do not use another board's exposed eMMC clock/test-point procedure without electrical qualification of this MPC One revision.

### 8.6 F — factory p6 replacement

Replacing p6 with a fully compatible MPFree Linux and `/boot` might allow installed U-Boot to boot MPFree directly without ever launching the factory application or Linux. But this would overwrite the only normal rootfs, not install a proven dual-boot system. It depends on exact kernel/DTB/boot args, correct partition geometry and a **tested** p6 restoration method. It is a deferred, higher-risk option, even with a complete private disk image.

### 8.7 The preferred eventual design

**Proposed:** Preserve factory `p1`/`p3` and a known-good recovery path, while making a separately stored MPFree image the selected normal kernel/rootfs. Use an A/B or equivalent health-confirmed selection mechanism that never loses the last known-good factory boot. Once genuine silicon-ROM recovery has been qualified, evaluate whether replacing first-stage components adds sufficient value. Separate the *bootloader* project from the *Linux userspace* project so either can evolve without risking the other.
