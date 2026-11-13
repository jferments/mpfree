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

