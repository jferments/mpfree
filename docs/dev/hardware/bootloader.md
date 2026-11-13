# The MPC One bootloader: from RK3288 reset to the Linux kernel

**Technical reference for MPFree developers · Original MPC One (Gen 1) · Updated 13 November 2026**

This document explains the bootloader as a working system, not simply as a list of partitions and function addresses. Its purpose is to give an operating-system developer enough understanding to decide **where an independent kernel could enter the boot process, what the stock bootloader already provides, and what would actually be at risk when changing the boot path**. It describes the original MPC One with the Rockchip RK3288 processor and the bootloader preserved in the owner’s firmware 3.6.0 eMMC backup. It is not a specification for every Akai MPC model, nor proof that firmware 3.10.1 has installed an identical early bootloader.

Read this alongside the [complete Linux and operating-system architecture](mpc-one-boot-architecture.md), which describes the filesystem, kernel, hardware drivers, firmware updates and prospective MPFree installation model. The [original November evidence records](archive/2026-11/README.md) retain exact measurement history.

## 1. Why the bootloader matters

An MPC One begins working before Linux is involved. Pressing the power button causes the RK3288 to start a small program stored in silicon. That program does not know what the MPC application is, what an ext4 filesystem is, or where the samples are kept. It only knows how to find a sufficiently recognizable first-stage loader on supported boot media, move it into internal memory and begin executing it. The first-stage loader then prepares the processor and external RAM well enough to run the larger U-Boot program. U-Boot understands disks and filesystems, locates a Linux kernel and device tree, and starts Linux. Linux finally mounts the factory filesystem and launches system services; only then does the MPC software appear.

This ordering is fundamental to MPFree. If we replace the factory music application, the factory bootloader and Linux kernel still run first. If we use `kexec` to launch our own kernel, factory Linux must first be sufficiently operational to request the handoff. If we want MPFree to start directly on power-on, we need an entry **at or before U-Boot’s selection of the Linux kernel**, or a separate first-stage boot path. Those are more demanding tasks because they happen before ordinary Linux recovery tools exist.

A useful mental model is that the device contains *several computers in succession*. The BootROM is a tiny, rigid loader. The SRAM stage is a hardware initialization program constrained by memory. U-Boot is a comparatively capable pre-Linux operating environment. Linux is the general-purpose kernel. Each later stage has more services but depends on the earlier stages functioning.

```text
RK3288 reset
   │
   ▼
SoC BootROM                     immutable, in silicon
   │ finds ID block at LBA 64
   ▼
Rockchip ID block + SRAM stage   eMMC p1; code near 0xFF704000
   │ initializes boot hardware and loads second stage
   ▼
U-Boot proper                    eMMC p3; configuration in p2
   ├─ recovery condition ──────► USB Fastboot Update Mode
   └─ normal boot scan
         │
         ▼
   p6:/boot/boot.scr.uimg
         │ loads zImage and rk3288-az01-acva.dtb
         ▼
     Linux bootz
         │
         ▼
     Linux / systemd / MPC app
```

The labels in the diagram are confirmed parts of the reference image, but not every branch has been exercised experimentally. In particular, U-Boot’s normal filesystem scan and its Fastboot RAM `boot` callback are visible in stored software; a complete owner-kernel boot through those functions has not yet been performed on the physical MPC.

## 2. The physical storage from the bootloader’s perspective

The MPC’s internal eMMC exposes a conventional block-device user area and two additional eMMC hardware boot areas. Linux calls the user area `/dev/mmcblk0`. The full user area on the reference device is 7,820,083,200 bytes, or approximately 7.28 GiB. Although eMMC is flash internally, both the boot stages and Linux normally address this user area as a linear sequence of **512-byte logical sectors**.

The important sectors are at the *beginning* of that address space. The eight GPT partitions are not interchangeable with eMMC’s separately exposed `boot0` and `boot1` areas. On the measured MPC, both hardware boot areas were 4 MiB of zero bytes, so the relevant installed first-stage loader is in the user-area partition `p1`.

| eMMC user-area location | GPT name | Exact start | Size | What is there |
|---|---|---:|---:|---|
| Partition 1 | `uboot-spl` | LBA 64 | 64 KiB | Rockchip ID block and SRAM first stage |
| Partition 2 | `env` | LBA 192 | 8 KiB | U-Boot environment |
| Partition 3 | `uboot` | LBA 256 | 384 KiB | U-Boot proper, stored as a legacy uImage |
| Partition 4 | `splash` | LBA 1,024 | 4 MiB | Startup graphics |
| Partition 5 | `recoverysplash` | LBA 9,216 | 4 MiB | Recovery/update graphics |
| Partition 6 | `rootfs` | LBA 17,408 | 500 MiB | ext4 factory OS, including Linux kernel and DTBs |
| Partition 7 | `content` | LBA 1,041,408 | 4 GiB | Factory content |
| Partition 8 | `data` | LBA 9,430,016 | ~2.79 GiB | Projects, persistent settings and overlays |

An LBA is a **logical block address**. LBA 64 means 64 × 512 = 32,768 bytes from the start of the eMMC user area. It does **not** mean byte offset 64 or the start of eMMC’s boot0 area. Partition 1 begins at byte offset `0x8000`; partition 3 begins at byte offset `0x20000`. These are board-image facts useful to a loader inspector, but not reasons to write those sectors.

The GPT itself, including its primary and secondary copies, is part of the full user-area image. U-Boot also stores a GPT layout template in its environment. That template demonstrates the board’s partition naming conventions and possible repair facilities, but does not guarantee that invoking a GPT-write command would preserve the original disk.

For an installer, the partition distinction establishes three different levels of consequence. An isolated file written to removable media is relatively easy to undo. Changing a file in the factory rootfs affects Linux’s startup but can potentially be repaired while U-Boot still runs. Changing `p1` or `p3` can prevent even U-Boot Update Mode from starting, at which point the normal desktop updater may no longer help. The original eMMC backup is real and verified; an independent way to restore it after an early-loader failure is still being qualified.

## 3. Stage zero: the silicon BootROM

The RK3288 begins with code permanently manufactured into the processor. Its responsibilities are deliberately narrow: initialize enough internal resources to read a boot medium, recognize a Rockchip-compatible first-stage image, and hand off to the code in that image. On RK3288 platforms, the usual disk image convention puts a Rockchip ID block at sector 64. This is why the MPC’s `uboot-spl` partition starts there rather than at sector zero.

The BootROM is independent of any Linux version and independent of the MPC application. A normal firmware update may replace filesystems or later boot code, but it does not replace the silicon ROM itself. This provides an attractive theoretical recovery foundation: the processor has mechanisms for accepting a small loader even when normal eMMC startup cannot proceed.

There is an important caveat. A ROM mechanism is only useful if we can actually **enter its recovery state on this PCB** and demonstrate that it executes an independently built loader. The reference MPC’s valid eMMC ID block makes it unnecessary for the normal boot process to enter USB ROM mode. We have **not** demonstrated a reliable software request for that mode, a qualified physical entry mechanism, or an owner-built SRAM program executing through that path. Other RK3288 boards provide valuable design references but cannot establish this unit’s electrical wiring or recovery behavior.

### 3.1 Two different boot orders

The silicon ROM’s selection of a first-stage boot medium and U-Boot’s later search for a Linux kernel are completely different operations. The ROM may prefer a recognized first stage on internal eMMC, while U-Boot may have a configured list of filesystem devices. An SD card containing a valid `zImage` is not itself an alternative Rockchip first-stage image. Conversely, an SD card prepared with a Rockchip loader does not guarantee the ROM will choose it while a valid eMMC loader is present.

This distinction is easy to miss when designing a “boot from SD” feature. An SD card can be used at **two** possible stages: it could contain an early loader recognizable to the ROM, or it could contain a kernel and filesystem recognizable to *already-running U-Boot*. Each approach has different boot-order, power-init, and recovery requirements.

### 3.2 ROM USB mode is not Akai Update Mode

The BootROM’s USB protocol is associated with the RK3288 silicon and its SRAM-loading facility. Akai’s Update Mode, by contrast, is implemented by installed U-Boot. Seeing the MPC enumerate over USB in its familiar desktop update workflow demonstrates that *some* loader and the U-Boot USB stack work; it does not demonstrate access to silicon-ROM recovery.

The installed software also contains a Rockchip Fastboot reboot-mode value, discussed later. That value has been traced to a code path that selects Fastboot-oriented behavior. It should **not** be interpreted as a confirmed route into silicon ROM USB service.

## 4. Stage one: the Rockchip ID block and SRAM loader

The first-stage image begins in the 64-KiB `p1` partition. The opening 512-byte block uses a historical Rockchip header encoding. When decoded with the format defined by Rockchip/U-Boot tooling, its signature is `0x0FF0AA55`. Its `disable_rc4` field equals `1`, meaning the **initial executable payload** is stored plainly. The header encoding is a format detail, not an assertion about whether some other boot step applies an integrity policy.

The most important header fields are:

| Field | Value in the reference image | Meaning |
|---|---|---|
| Signature | `0x0FF0AA55` | Recognized Rockchip legacy ID-block header |
| `init_offset` | 4 sectors | First-stage payload begins 2,048 bytes after p1 start |
| `init_size` | 64 sectors | 32 KiB allocated to the initial stage |
| `init_boot_size` | 1,088 sectors | 544-KiB boot-chain size/allowance field, not the size of p1 |
| `disable_rc4` | 1 | First-stage executable data is not RC4-encoded |

The larger `init_boot_size` has sometimes been mistaken for proof that the first-stage partition contains more than half a megabyte of code. It does not. The partition is 64 KiB and the first-stage region is 32 KiB; the larger header value is consistent with a U-Boot Rockchip packaging convention that provides allowance for subsequent boot data. The second-stage U-Boot image actually begins in a distinct partition at sector 256.

Within `p1`, the executable stage starts at offset `0x800` with the four-byte `RK32` identifier. It is loaded into the RK3288’s internal SRAM at approximately `0xFF704000`. Its ARM instructions begin after the marker, and a reset vector at the stage-relative offset `0x20` branches into the initialization sequence. This code cannot rely on a fully configured external DDR subsystem because helping establish that subsystem is part of the early loader’s job.

The internal organization is useful when checking an image or debugging early boot:

```text
p1 + 0x0000   encoded Rockchip header
p1 + 0x0200   reserved / padding region
p1 + 0x0800   "RK32" first-stage marker
p1 + 0x0804   initial stage bytes
p1 + 0x0820   ARM reset vector
               maps to SRAM 0xFF704020
p1 + 0x0864   reset-vector branch target
               maps to SRAM 0xFF704064
...
p1 + 0x8800   end of the nominal 32-KiB stage
```

The initial ARM vector passes through code near `0xFF704064` and `0xFF7040B4`, with a later initialization region around `0xFF705208`. The first-stage code contains functions associated with iterating possible boot devices near `0xFF705350` and reading an MMC image near `0xFF7054A4`. The latter ultimately reaches eMMC LBA `0x100` (= 256), where the second-stage U-Boot image begins.

These addresses are **reference-image details**, not a fixed application binary interface. The first-stage may use both ARM and Thumb instructions, literal tables and embedded data; a memory range is not necessarily a continuous run of instructions. They are documented to explain what the current loader does and to support validating a future independent implementation, not as hard-coded jump targets for MPFree.

### 4.1 How the first stage recognizes U-Boot

The installed second stage is packaged with a legacy U-Boot `uImage` header, whose on-media magic is `0x27051956`. The first-stage image contains the corresponding magic value and header-handling logic. Code associated with `0xFF7052C0` compares the first block of the candidate next-stage image to that magic and interprets big-endian fields such as the payload size, load address and entry address.

This helps explain why the storage arrangement is robustly stage-based. The BootROM needs only a small recognized first stage. That stage loads the much larger U-Boot image from a known raw-sector location and prepares its entry. The first stage does not need to mount ext4, understand MPC projects or find Linux by filename. Those features belong to the later bootloader.

The U-Boot image header can be inspected independently, including its metadata and CRC32 fields. A matching CRC indicates that the bytes agree with the stored checksum; it is **not** a cryptographic authenticity guarantee. Accepted image types and any other board policy should be evaluated separately.

### 4.2 Where the first-stage handoff becomes uncertain

We have a good account of the image container, early ARM control flow, boot-device iteration and raw next-stage loading. The complete last handoff, every fallback branch, and the exact response to all candidate boot media are not fully demonstrated by device-origin traces. That distinction matters if MPFree eventually implements its own SRAM stage. It must reproduce the required DRAM, clock, pinmux and storage initialization for *this board*, not merely copy the observed jump addresses.


## 5. Stage two: U-Boot proper

The second stage is U-Boot 2016.05 with inMusic board additions. Its role is qualitatively different from the SRAM stage. By the time it runs, enough of the SoC has been initialized to use external RAM, execute a larger codebase, communicate over USB and serial, inspect block devices and read files from supported filesystems. That makes it the first part of the normal boot chain that has the *kind* of functionality MPFree eventually needs for a dual-boot manager.

The U-Boot image occupies the GPT `uboot` partition, `p3`, beginning at sector 256. It begins with the four-byte legacy uImage signature `27 05 19 56` and contains image metadata plus the actual executable payload. The payload has been identified as ARM code with Thumb callback functions. Legacy image inspection establishes the container header, expected payload size and checksums; image validation on a workstation does not itself prove every instruction will execute on a particular board.

One especially important property is that U-Boot is **not** the Linux kernel and does not need the p6 root filesystem to exist before it can begin its own work. Provided that the BootROM, SRAM stage, U-Boot image and necessary hardware configuration are functional, U-Boot can run its commands even if p6 has an invalid ext4 filesystem. That is why the factory Update Mode is a plausible rootfs recovery path. It is *not* sufficient for a failure that prevents U-Boot itself from loading.

### 5.1 The environment: configuration rather than executable code

U-Boot stores persistent settings separately from its executable image. On this MPC that storage is the tiny `env` partition, `p2`, at LBA 192. Linux can read it through the installed `fw_printenv` utility, whose location/configuration is described by `/etc/fw_env.config`. It is helpful to think of the environment as a small, persistent pre-Linux configuration database: it contains scripts and variables describing what to boot, in what order, and what to do when something fails.

The significant reference values are:

```text
bootcmd=run az01_bootcmd

az01_bootcmd=if test x${dorecovery} = x1; then
                 run az01_recovercmd;
             fi;
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
```

The human-readable wrapping above preserves the logic; the environment stores commands as strings rather than a shell script file with these exact line breaks. The essential condition is `dorecovery`. When the running board logic supplies a value selecting recovery, `az01_bootcmd` invokes `az01_recovercmd` and enters Fastboot. Otherwise it invokes the distro boot scan. After all normal boot targets fail, the script calls `reset_bootloader`. We know that command and its associated reset logic exist; what the complete failure cycle does in every state is not established.

The setting `bootdelay=0` means the user does not normally see a pause at a U-Boot prompt. The environment identifies serial and GPIO-key input, but an appropriate board-specific serial interface, voltage level and actual boot-interrupt behavior must be established before claiming a useful interactive console. In the device tree the console corresponds to UART2 at 115200 baud. UART access would be valuable for observing cold boot without touching flash; electrical mapping of a safely accessible header remains a separate hardware task.

Two values, `fdtfile` and `dorecovery`, were not found as ordinary saved variables in the recorded environment. Their effect is visible in the boot process, so board-specific initialization is a plausible source. They should not be treated as owner-editable persistent settings until their initialization sequence is verified.

### 5.2 What `distro_bootcmd` actually does

U-Boot’s distro boot conventions let a board describe a list of storage interfaces and candidate file locations rather than hard-code a single kernel image in its executable. The MPC’s environment sets `boot_targets=mmc0 mmc1` and looks under `/` and `/boot/` for boot scripts such as `boot.scr.uimg` and `boot.scr`. Where U-Boot’s build supports it, an `extlinux/extlinux.conf` file is another conventional boot description.

It would be tempting to read “mmc0 mmc1” as “eMMC then SD” and conclude that an inserted SD card can override the installed OS. This is premature for two reasons.

First, Linux device names and U-Boot device numbers are produced by different software. The Linux device tree associates the internal eMMC controller with `ff0f0000` and removable SD with `ff0c0000`; the U-Boot `mmc` enumeration needs its own board-level confirmation. The names are likely aligned, but that is not yet a measured U-Boot bus map.

Second, if the **first** target contains a bootable script and kernel, U-Boot normally has no reason to inspect the second. On a working MPC with a valid factory p6, simply inserting an SD card is therefore unlikely to select it automatically. This is a selection-policy problem, not evidence that the SD controller or U-Boot’s filesystem reader is incapable of loading Linux. A future MPFree boot manager must distinguish “U-Boot can read SD” from “U-Boot will choose SD while factory boot is healthy.”

An effective direct-boot system needs one of three mechanisms: a verified existing user-controlled boot selection before distro scan, a **temporary** boot command supplied through a qualified pre-Linux interface, or a persistent boot policy that can be recovered if it points at a missing/corrupt image. The first two are preferable for development because they avoid modifying `p2`.

### 5.3 The stock kernel is a filesystem file

The factory Linux kernel is not in a raw kernel partition. U-Boot reads it out of the rootfs ext4 partition `p6`. On the reference device, the relevant files include the Linux `zImage`, the board’s flattened device-tree blob (DTB), and `/boot/boot.scr.uimg`. That means the stock bootloader must understand enough of the filesystem to load several named files rather than simply jumping to a hard-coded disk offset.

The 535-byte U-Boot script performs the last part of the job. It obtains the ARM kernel image and the chosen FDT, applies a board-specific overlay when appropriate, sets up the kernel command line and calls `bootz`. The reference rootfs contains several closely related DTBs, but the tested MPC One uses `rk3288-az01-acva.dtb`. A related `rk3288-az01-acva-c.dtb` is not an interchangeable cosmetic variant: it includes different CPU-frequency information. Selecting the wrong DTB can change device initialization or make warm kernel handoff fail unexpectedly.

The factory command line included a root PARTUUID, `rootwait ro`, `rfkill.default_state=0`, `fsck.repair=yes` and `isolcpus=2-3`. The PARTUUID identifies the factory rootfs independently of any temporary Linux `/dev/mmcblkN` numbering. The `ro` flag directs Linux to mount the root read-only during early startup. `isolcpus=2-3` is part of the factory real-time scheduling approach, but its effect should not be extrapolated into a complete description of the audio thread's scheduling or CPU ownership.

U-Boot also adjusts the live FDT. During the 3.10.0 alternate-kernel tests it was necessary to preserve relevant live hardware information and account for reserved-memory regions and board-specific PHY behavior. Therefore an MPFree installer should not naively copy a random DTB and assume it is equivalent to the tree given to factory Linux at a cold boot.

### 5.4 Image formats and the meaning of a valid header

The installed U-Boot understands its legacy uImage container. Later code paths also recognize FDT/FIT-style structures, which are introduced by the big-endian `0xD00DFEED` FDT magic. The observed legacy-image handling includes routines consistent with header checks and data CRC verification, along with informational printing of the image’s OS, architecture, compression, size, load address and entry address.

These checks answer different questions. An image with the right **format** can still be incompatible with the CPU. An image with matching **CRC32** can still target the wrong memory address. An ARM Linux kernel with the right **load address** can still fail because it receives the wrong DTB. And image-format acceptance says nothing by itself about all board-specific release or integrity policies. A usable MPFree boot image must satisfy every relevant condition and then actually start on hardware.

## 6. U-Boot Update Mode and its USB interface

The MPC’s familiar firmware update entry is implemented before Linux. The U-Boot environment explicitly defines `az01_recovercmd` as `fastboot 0`. The board code contains recovery-selection logic, including an analog key input comparison. Some observations suggest that ADC readings below 100 select a recovery path. The exact mapping from the physical key combination to the ADC channel and internal state word is not fully proven, so a developer should describe the *capability* without claiming that every register in the selection path is completely characterized.

This design has a practical consequence: the USB firmware updater does not require the p6 Linux root filesystem to start before the board can communicate. If a factory kernel is missing or the rootfs is unreadable, U-Boot may still be able to receive a manufacturer update. That makes Update Mode an important part of the recovery plan for MPFree. It is not yet a fully rehearsed recovery guarantee on the reference MPC, and it depends on intact first-stage and second-stage bootloaders.

The pre-Linux Fastboot command table contains **eight registered command prefixes**, each associated with a Thumb callback:

| Command prefix | Purpose implied by handler | Verified callback code address |
|---|---|---|
| `reboot` | Reset device from the USB session | `0x00122CF8` |
| `getvar:` | Return bootloader/device variables | `0x00122D38` |
| `download:` | Receive an image or data into RAM | `0x00122980` |
| `boot` | Schedule execution of a downloaded image | `0x00122964` |
| `continue` | Leave the Fastboot session/resume boot path | `0x00122948` |
| `flash` | Storage update operation | `0x00122C5C` |
| `erase` | Storage erase operation | `0x00122BF8` |
| `oem` | Manufacturer-defined commands | `0x00122854` |

The low bit of each stored function pointer indicates Thumb instruction state; the addresses in the table are the corresponding instruction addresses. These values come from the particular original bootloader, not a published API. The presence of handlers makes each path a genuine part of the compiled U-Boot, but it does not mean each operation is necessarily enabled or safe in every runtime state.

### 6.1 Why the `boot` command is especially interesting

For MPFree, `download` and `boot` are the most promising pre-Linux functions. If the bootloader can accept a correctly packaged kernel into RAM and then execute it, an owner could potentially start an independent Linux image **without booting factory Linux and without writing internal eMMC**. This would be a major improvement over the current `kexec` pathway.

The two operations are distinct. `download` receives data; `boot` asks the bootloader to interpret and execute an image that has already been received. The recorded code shows that the immediate `boot` command handler does not jump to the kernel directly. Instead, it installs a USB request completion callback, sends the protocol response and then executes the follow-up handler after the USB response has been transmitted. That ordering matters because an immediate kernel jump would prevent the USB host from learning whether its request was accepted.

The `boot` completion code constructs a short argument array and invokes a function consistent with U-Boot’s `bootm` command path. The underlying image classifier understands legacy image structures and FDT/FIT headers. This establishes that U-Boot has a meaningful execution pathway; it does **not** establish an accepted complete MPC One kernel/DTB/ramdisk combination. We have not sent a real `fastboot boot` test image to the reference MPC and watched it become a functioning MPFree Linux system.

### 6.2 How the data reaches memory

The Fastboot download implementation contains an image buffer base near `0x00800800`. Separate RAM state tracks the requested length and the amount received. Each incoming USB request is copied into the buffer after the previous bytes, subject to the code’s length checks. The decoded implementation includes a nominal download limit of `0x70000000` bytes.

That value is **not** a recommended transfer size. A theoretical limit in a parser is not a memory allocation plan. Actual safe RAM occupancy depends on U-Boot’s own text/data/stack, USB DMA, FDT, reserved regions, kernel decompression workspace and Linux’s early boot placement rules. Our actual hardware has approximately 2 GiB of RAM, but this does not permit an arbitrary file to occupy 1.75 GiB without colliding with bootloader state. The first useful test should use the smallest known-good image whose memory layout can be checked thoroughly.

There are also state checks in some Fastboot callbacks for download and persistent operations. Their exact initialization and every decision branch have not been demonstrated through physical tests. As a result, the existence of a callback cannot be equated with unrestricted availability of the command in every maintenance session.

### 6.3 `continue`, `flash` and `erase` are not alternative names for boot

`continue` is a control-flow operation that allows the loader to leave the current maintenance session; it is not equivalent to executing the downloaded kernel. Its completion callback affects a Fastboot exit flag, but the final caller's exact return path has not been observed on the reference unit.

`flash` and `erase` are persistent storage operations. The firmware uses partition names that include `rootfs`, `content` and `data`, and some maintenance components understand bootloader regions. These commands therefore have completely different consequences from a RAM-only image transfer. An MPFree proof-of-concept should not issue state-changing storage commands in order to establish whether a USB connection or image parser works.

The custom `oem` subcommands belong to the manufacturer's maintenance interface. They have command-specific behavior, and no current MPFree installer design depends on exercising them. The useful task is to understand which *nonpersistent* facilities U-Boot already provides and whether they can boot a prepared open-source kernel.

### 6.4 How to distinguish USB states on a workstation

There are at least three related but distinct USB personalities:

**Akai U-Boot Fastboot** runs *after* the eMMC first and second stages and is therefore expected to remain accessible when Linux is damaged but U-Boot is healthy.

**RockUSB loader mode** is implemented by a loader or compatible U-Boot facility, not by the MPC application. Its particular capabilities depend on the loaded program; it should not be assumed to be the same protocol as Fastboot.

**Genuine RK3288 ROM USB mode** comes from silicon before any installed U-Boot. In appropriate circumstances, a compatible host can send an early SRAM payload. We have not yet demonstrated entry into that mode on this physical board.

A workstation listing a USB device cannot establish all of these details by vendor name alone. Look at the actual reported mode, expected protocol and reproducible behavior. In the MPFree project, the existence of U-Boot Fastboot must never be used as proof that independent ROM recovery is available.


## 7. The software reboot path: Fastboot is not silicon-ROM recovery

A Rockchip system may use a register in the system-control block to communicate a requested reboot mode to an early loader. The installed MPC first-stage code refers to register `0xFF730094` and the Fastboot-related value `0x5242C309`. A function in the existing loader writes that value before resetting the processor. U-Boot exposes a `reset_bootloader` command, and its environment calls it when normal boot targets fail.

This is an instructive result because it shows that some boot-mode transitions happen **below Linux**, within the installed early loader. It also explains why the same board can offer both an ordinary boot and a pre-Linux maintenance session without separate firmware filesystems. But the code supports a **Fastboot-oriented reboot**. No corresponding, confirmed source-level path has been demonstrated from an owner-accessible Linux command into genuine BootROM USB service.

On some Rockchip systems a BootROM download request uses a different register value, conventionally `0xEF08A53C`, and other loader-mode values exist. The observed p1 image did not contain exact little-endian word matches for the common BootROM-download or RockUSB-loader values in the surveyed ranges, while it did contain the Fastboot value. Absence of an exact literal is not absolute proof that the mode is impossible: code can derive a value or use a different path. It is, however, a reason **not** to present a hypothetical ROM entry as working on the MPC One.

If an independent bootloader is to remain usable after later Akai firmware changes, it must have a recovery path that does not depend on these vendor-specific reset functions. That is the rationale for eventually qualifying a real SRAM-first BootROM entry method.

## 8. What can actually be done without changing the factory boot path?

The lowest-level **proven** method for starting another Linux kernel on this MPC is currently `kexec`. That facility belongs to *running Linux*, not U-Boot. It can replace the kernel in RAM, and our 3.10.0 tests proved it works on real hardware without writing the factory boot partitions. Because it needs factory Linux to start, it does not satisfy the ultimate independent cold-boot requirement.

U-Boot’s RAM `boot` and removable-media loading features are promising **one stage earlier**. If they can be exercised under a no-write policy, a developer could start a kernel before any factory Linux instructions execute. No such complete test has yet been demonstrated, so MPFree must treat these as experiments requiring a safe, checked image and a known power-cycle fallback.

The following distinctions guide development:

| Entry point | Requires factory Linux first? | Requires intact factory U-Boot? | Can avoid eMMC writes? | Physical result |
|---|---|---|---|---|
| Run MPFree app under existing kernel | Yes | Yes for normal boot | Yes, session-only | App components tested |
| `kexec` to another Linux kernel | Yes | Yes for initial boot | Yes, with MMC isolated | **Three successful 3.10.0 boots** |
| U-Boot removable-media boot | No, if selected | Yes | Potentially | Selection not proven |
| U-Boot Fastboot RAM boot | No, if image accepted | Yes | Potentially | Kernel execution not proven |
| Independently loaded BootROM SRAM stage | No | No | Potentially | ROM entry/execution not proven |
| Replace p6 rootfs | No | Yes | **No; factory p6 changes** | Not an approved first installer path |

### 8.1 A first practical milestone: read-only U-Boot characterization

Before trying a replacement kernel, verify device enumeration and file-loading semantics without persisting environment changes. The exact U-Boot `mmc` mapping and boot-script lookup order matter more than the nominal availability of those commands. If we can demonstrate that the bootloader can read an SD card and start a *temporary* user-supplied kernel while leaving the factory installation untouched, the project can graduate from a Linux-mediated `kexec` workflow to direct pre-Linux development.

Such a trial must establish which image types are accepted, where they are loaded, whether the correct FDT is supplied, whether the board’s clocks and peripherals initialize correctly, and what happens when the image or removable medium is absent. These are observable properties, not things to infer from a command name.

### 8.2 Why “just change boot_targets” is not safe enough

The environment might appear to offer a simple solution: change `boot_targets` so SD is scanned before eMMC. On a healthy machine that could make a removable image win the boot race. But `p2` is a persistent boot-critical partition, and a bad selection policy can leave the MPC without a normal startup even though both kernel images still exist. Recovery would then depend on the remaining key-based or USB maintenance mode, whose restoration behavior has not been qualified comprehensively.

A complete installer should ultimately use a versioned, health-confirmed boot selection scheme. It would attempt a candidate MPFree image, mark it healthy only after successful startup, and automatically retain or restore a known-good factory choice if the attempt fails. That is a *design goal*. It should not be simulated by untested edits to an 8-KiB U-Boot environment on the only reference board.

### 8.3 What a direct MPFree kernel must contain

A successful pre-Linux handoff needs more than an ARM `zImage` file. The kernel must be the right architecture and memory format; it needs the exact board device tree and boot arguments; its root filesystem must reside on a device it can initialize early; and essential firmware/driver support must be available before the user interface appears.

The 3.10.0 temporary boot tests also showed that even with a correct kernel and FDT, **warm** device state can matter. Ethernet initialization failed on its first attempt after `kexec` until the RTL8211E PHY was appropriately reinitialized and the interface brought up again. A cold U-Boot boot may have different hardware state and therefore needs its *own* acceptance test. A kernel that runs under `kexec` should not automatically be declared U-Boot-ready without verification.

## 9. Recovery architecture and the protection of early boot storage

The primary recovery principle is simple: **do not make the only known-good startup path dependent on an experimental replacement**.

A factory root filesystem can plausibly be repaired while the installed first-stage and U-Boot still work. An invalid U-Boot image, however, means the `fastboot 0` code inside it may never run. Recovery then moves downward to an independent first-stage or silicon-ROM mechanism. That is why a known complete eMMC backup, though essential, is only the beginning of recovery engineering.

The project has a verified owner-held backup of the entire 7,820,083,200-byte eMMC user area and both hardware boot areas. The original study checked device-side hashes against the backup, including boot-critical p1/p3–p6 regions. The two 4-MiB hardware boot areas contained only zero bytes. This supports accurate restoration **provided an independent means of writing the bytes is available**. That last condition remains open.

ROM USB recovery would ideally start with a tiny, independently compiled SRAM program requiring no eMMC contents. After proving it executes, the project could bring up DRAM, a USB communication path and an eMMC read interface. A successful **complete read-back** compared to the private backup would be a strong next qualification. Only after a separate, carefully prepared restore rehearsal could the boot-critical partitions be considered serviceable.

### 9.1 Why security-state observations are not enough

The reference unit’s Linux NVMEM observations reported blank fields associated with a secure-boot enable value, trusted-key hash and lock state in a Rockchip secure eFuse bank. This supports—but does not prove—the hypothesis that the relevant silicon policy is unprovisioned on this device. The precise silicon mapping, electrical programming supply on the actual board and real ROM acceptance of an owner-built SRAM image still need independent confirmation.

A later firmware package also includes software referring to bootloader signatures. That component does not, simply by existing, prove the MPC One’s silicon ROM currently requires a vendor-signed first stage or that an update programs one-time hardware state. Both optimistic and pessimistic interpretations must be checked against physical behavior. A recovery strategy should not rest solely on strings in a utility or the apparent absence of a register bit.

### 9.2 Scope of recoverability

| Failure | What would still run? | Candidate recovery | Qualification |
|---|---|---|---|
| Temporary MPFree kernel hangs | BootROM, stock p1/p3/p6 unchanged | Power-cycle | **Observed repeatedly** |
| MPFree application service fails | Factory Linux still present | Restart service / power-cycle | Depends on service handoff test |
| Factory rootfs p6 is invalid | BootROM and p1/p3 should still start | U-Boot Fastboot + official update | Architecture verified; restoration rehearsal outstanding |
| U-Boot environment p2 is invalid | BootROM and initial stage may run | Board-dependent default/recovery behavior | Not comprehensively qualified |
| First stage p1 or U-Boot p3 invalid | Normal pre-Linux chain may stop | Silicon-ROM owner recovery | **Not established on reference hardware** |
| eMMC boot config or OTP changes | Depends on hardware state | Specialized board-aware service | Not qualified |

Do not generalize “power cycle restores stock” beyond *those tested session-only operations that left eMMC unchanged*. It cannot undo an erased partition, changed boot configuration, or one-time hardware state.

## 10. Recommended roadmap for the bootloader implementation

The next release does **not** need a custom SPL. MPFree already has a working path to a separate Linux kernel using `kexec`. That path is well suited to building the actual OS: initialize display, audio, MIDI, inputs, persistence and independent userspace; test it repeatedly without giving the new system control of internal storage.

In parallel, qualify the **factory U-Boot** as a possible temporary pre-Linux launcher. Confirm board enumeration and accepted image formats through observations that do not modify boot-critical state. The first substantial breakthrough would be a confirmed boot of an MPFree kernel from RAM or removable media *before the factory Linux kernel starts*, with reproducible power-cycle fallback.

The next independent milestone is recovery: demonstrate real silicon-ROM entry on the board, execution of a source-built SRAM stage, and read-back of the original eMMC without the stock loader. Only then consider changing persistent U-Boot selection or replacing p6. If the long-term design uses an entirely owner-written SPL and U-Boot, the same recovery requirements apply even more strongly.

Once both direct boot and recovery are proven, implement a small explicit **factory/MPFree boot contract**. It should specify what stores the last known-good selection, how the new image is checked before a trial boot, how health is acknowledged after startup, how power loss affects the selection, and how the user selects the factory environment even when MPFree is faulty. This is more important than any particular splash screen or menu implementation.

## 11. Evidence register and related documentation

The concrete source for the original partition layout, environment, image locations and checksums is the [archived boot map](archive/2026-11/boot-map.md). The first-stage header, SRAM mapping, U-Boot callbacks and image-format notes are retained in [the detailed boot-system source record](../research/bootrom.md). The [archived ROM recovery plan](archive/2026-11/owner-rom-recovery-plan.md) explains the uncompleted independent-recovery prerequisites. The [archived USB and ROM notes](archive/2026-11/maskrom-usb.md) distinguish device modes and physical-port evidence. The [archived temporary kernel boot history](archive/2026-11/kexec-test-boot.md) documents the actual 3.10.0 hardware trials.

For the broader runtime environment, see [MPC One Linux and Operating-System Architecture](mpc-one-boot-architecture.md); for installation policy, see [Installer and Recovery Contract](installer-recovery-contract.md); for the current firmware review, see [MPC OS 3.10.1 Compatibility](firmware-3.10.1-compatibility.md).

**Current bottom line:** We understand the normal bootchain well enough to build compatible kernel images and to identify credible independent-boot interfaces. We have verified the no-eMMC-write `kexec` bridge. We have **not** yet earned the right to describe arbitrary U-Boot RAM boot or silicon-ROM recovery as a completed, recoverable on-device workflow. The next phase should turn those particular hypotheses into repeatable measurements while preserving the known-good factory boot.
