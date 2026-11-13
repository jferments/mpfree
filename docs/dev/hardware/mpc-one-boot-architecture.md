# MPC One operating-system architecture: from power-on to a free Linux system

**MPFree engineering reference · Original MPC One (Gen 1, Rockchip RK3288) · 13 November 2026**

The purpose of this document is to explain how the MPC One actually works as a computer, and how that knowledge can be used to build an independent, free and open-source operating system for it. The intended reader understands Linux and software development but may never have seen the internals of an MPC. The emphasis is on *causality*: which component loads the next component, where it finds its files, which parts of the hardware it controls, what happens when a component fails, and how we can gradually substitute MPFree without losing the factory system.

The architectural picture is now fairly complete. We know the internal flash layout and the normal boot chain. We know that the factory music software is an application running under a conventional embedded Linux environment. We have established root-level developer access, identified the hardware interfaces and services, and successfully started another Linux kernel from removable storage without changing the factory boot partitions. We have also identified promising capabilities in the installed U-Boot bootloader that may eventually make it possible to start MPFree directly at power-on, without first starting factory Linux.

The remaining question is not whether the hardware can run another operating system—it can, and has. It is how to make an alternative system **the selected operating system at power-on**, with a fallback that still works if the alternative kernel or its storage is unusable. The fact that U-Boot contains a command for executing a downloaded image is valuable, but different from having tested that command on the real board. Likewise, the existence of a silicon-ROM recovery protocol is different from having a tested independent recovery procedure for this particular MPC. This document keeps those differences explicit.

A second, more focused reference, [The MPC One bootloader](bootloader.md), goes further into the Rockchip ID block, the first-stage SRAM loader, U-Boot’s environment, raw sector layout, image formats and Fastboot command handlers. The material below gives the complete operating-system context needed to use those findings.

## 1. How to think about the MPC One

It is useful to stop thinking of the MPC One as a special-purpose music appliance with a mysterious, indivisible firmware image. Internally it is an embedded Linux computer based on a Rockchip RK3288 system-on-chip. It has four 32-bit ARM cores, approximately 2 GiB of memory, internal eMMC flash, an SD-card controller, USB, Gigabit Ethernet, a touchscreen, hardware buttons and pads, audio interfaces and a graphics subsystem. Linux provides the familiar mechanisms for controlling all of these: device drivers, device-tree hardware descriptions, block devices, filesystems, character devices, ALSA, DRM/KMS, evdev, networking and system services.

Akai’s music-production environment is a complex application layered on that system. It needs a suitable kernel, a set of drivers, a collection of userspace libraries, firmware for certain peripherals and services to initialize the board. But it is not itself the kernel, the first-stage loader or the filesystem. This is why our Rust applications can already communicate with MPC hardware and why a different Linux system can start on the same processor.

For MPFree there are **three distinct levels of independence**. We can run our own program on the factory kernel, after stopping or separating it from the MPC application. We can start our own kernel and userspace using the factory Linux kernel’s `kexec` facility, at which point the factory application and its kernel are no longer running. Finally, we can arrange for a bootloader to select our kernel *before* factory Linux runs. The first two levels are practical now; the third is the subject of the current bootloader work.

### 1.1 What has been observed on the actual hardware

The reference machine is an original MPC One, with a device tree identifying `rockchip,rk3288`, `inmusic,az01` and `inmusic,acva`. The factory Linux release observed in the initial hardware sessions was:

```text
6.6.95-az01-2025-06-27-rt55
armv7l
SMP PREEMPT_RT
```

The system is an inMusic `az0x` distribution built on Yocto/OpenEmbedded Scarthgap. Its C library reports glibc 2.39. The program ABI is 32-bit little-endian ARM EABI5 hard-float; dynamically linked programs use a loader such as `/lib/ld-linux-armhf.so.3`. That is an important practical fact for building MPFree: binaries intended for an ordinary 64-bit ARM Linux machine cannot execute here. A Rust target such as `armv7-unknown-linux-gnueabihf` is appropriate, provided the dependencies and runtime ABI match the actual system.

The motherboard contains roughly 2 GiB of RAM and the system was observed without swap. A large amount of memory was locked during factory music operation, consistent with a realtime-audio workload, but the exact origin of every locked mapping is not established. The factory kernel's `PREEMPT_RT` configuration means much of the kernel is engineered to reduce scheduling latency; it does not mean every audio or graphics function runs in a realtime thread or that an arbitrary MPFree process automatically receives the correct scheduling priority.

The reference unit had factory firmware 3.6.0 when the baseline was collected. It was subsequently modified to make a key-authenticated SSH service persistent; the modifications are recorded as `MOD-001` and `MOD-002` in [the preserved device-change record](archive/2026-11/device-modifications.md). Consequently the complete reference rootfs is *not* identical to an untouched factory 3.6.0 partition. That distinction matters when using partition hashes to determine whether a later test wrote anything.

### 1.2 A high-level map of the computer

```text
                   Original MPC One hardware
 ┌──────────────────────────────────────────────────────────┐
 │ RK3288: ARMv7 CPUs, DDR RAM, GPU, memory controllers     │
 │ Internal eMMC   SD slot   USB host + USB device/OTG       │
 │ Gigabit Ethernet   audio/MIDI   touchscreen/pads/keys    │
 └────────────────────────────┬─────────────────────────────┘
                              │ power-on
                              ▼
             silicon BootROM → SRAM first stage
                              │
                              ▼
              factory U-Boot / boot selection
                  │                      │
          normal kernel boot       USB Update Mode
                  │
                  ▼
         factory Linux 6.6 PREEMPT_RT
                  │
          mounts root/data/overlays
                  │
                  ▼
       systemd, ALSA/DRM/input/network, MPC app
                  │
             optional kexec
                  │
                  ▼
             MPFree Linux
         (USB root, own services, own app)
```

The chart makes the dependency explicit: when `kexec` transfers to MPFree Linux, we already have a different running kernel, but we still used the factory power-on boot path to get there. An independent cold-boot feature must move our choice upward into the U-Boot or first-stage portion of the diagram.

## 2. What happens from pressing power to seeing the MPC application?

The power-on sequence consists of small programs handing increasing responsibilities to larger programs. The earliest code has the least memory and fewest services. Each stage prepares the system for the next.

The RK3288 silicon BootROM starts first. The normal reference-unit boot looks for a Rockchip-compatible first-stage image in the eMMC user area at logical sector 64. The image lives in partition `p1`, which is called `uboot-spl`. Its recognized Rockchip header precedes an approximately 32-KiB initial loader that executes in the processor’s internal SRAM at approximately `0xFF704000`. The initial loader is responsible for early hardware setup and loading a larger U-Boot image from internal storage. We have identified the actual header fields, its first executable region and the code handling the second-stage image.

U-Boot proper occupies `p3`, beginning at sector 256. Unlike the SRAM stage, U-Boot has enough functionality to read filesystems, identify boot candidates, load a Linux compressed kernel and its device-tree blob, and choose an update mode. Its persistent environment is in a separate 8-KiB partition, `p2`. The reference environment runs an `az01_bootcmd` script. When a recovery condition is present it enters U-Boot’s USB Fastboot mode; otherwise it executes a normal distro-style boot scan with targets listed as `mmc0 mmc1`.

For normal startup U-Boot selects the factory root filesystem partition `p6`, reads the kernel and a matching board DTB from its `/boot` directory, and calls `bootz`. There is no dedicated kernel GPT partition on this device: the kernel is an ordinary file within the same ext4 filesystem that will become Linux’s root. The U-Boot script is named `/boot/boot.scr.uimg`, and the reference board's FDT is `rk3288-az01-acva.dtb`. The script constructs kernel arguments including the partition UUID of the root, mounts the intended root read-only, and passes board-specific configuration to the kernel.

The Linux kernel then takes over hardware initialization. It attaches the eMMC driver, mounts filesystems, initializes devices, starts `systemd` and brings up the services that make the MPC usable. Eventually `acvs.service` launches `/usr/bin/az01-launch-MPC` and then the main `/usr/bin/MPC` program. At that point the device appears to the user as an ordinary standalone MPC. All the earlier layers remain important, but none of them is the music application itself.

The exact bootloader organization, including the code-level evidence for its first-stage load, is documented in [bootloader.md](bootloader.md). For present purposes the key distinction is that **the factory rootfs begins well after both first-stage loaders**. A Linux failure does not necessarily mean U-Boot can no longer start. Conversely, changing the early loader is qualitatively more hazardous than replacing a Linux service.

### 2.1 U-Boot has real pre-Linux capabilities

U-Boot is not merely a stub that jumps to a fixed disk location. The image contains commands for MMC access, filesystem loading, script execution, kernel startup and USB maintenance. The reference environment includes:

```text
bootcmd=run az01_bootcmd
boot_targets=mmc0 mmc1
boot_prefixes=/ /boot/
boot_scripts=boot.scr.uimg boot.scr
az01_recovercmd=echo 'Starting fastboot...'; fastboot 0
bootdelay=0
```

Those settings make a removable-media kernel boot plausible because U-Boot already knows how to inspect multiple boot targets and start ARM Linux images. They do **not** prove that an inserted SD card is preferred over a healthy factory rootfs. The exact mapping of U-Boot’s `mmc0`/`mmc1` to this board’s eMMC and SD controllers still needs independent on-device confirmation, and the normal first boot target can win before the second is inspected.

The U-Boot Fastboot implementation also contains `download` and `boot` callbacks; the latter follows a code path consistent with executing an image from RAM. This is promising for MPFree because, if a compatible image is accepted, it could let a workstation launch our kernel without writing eMMC and without starting factory Linux. However, the image format, exact memory layout and actual command acceptance have not been tested as a complete physical boot. Until that is done, the proven nonpersistent kernel-launch method remains `kexec`.

### 2.2 Device trees are part of the boot ABI

The kernel cannot discover every board-specific wire automatically. An RK3288 tells us what processor peripherals exist, but it does not tell Linux which GPIO resets the Ethernet PHY, which display controller is connected to the touchscreen panel, or how audio is routed to the board’s codecs. Those relationships are represented in a device tree. On the MPC One, the bootloader chooses a board-specific flattened device tree blob and may modify it before handing it to Linux.

This matters for two reasons. First, board variants share similar filenames but are not necessarily identical. The reference MPC uses `rk3288-az01-acva.dtb`; another `-acva-c` variant carries different CPU frequencies. Second, the *live* tree can contain bootloader adjustments that are absent from the firmware file. During temporary-kernel development we had to preserve relevant board information, reserved-memory behavior and PHY reset configuration.

A robust MPFree boot package needs the right kernel *and* the right FDT, and should record which firmware/board observations justify that choice. A kernel built for an arbitrary RK3288 evaluation board is not enough.

## 3. How the eMMC is divided, and why the boundaries matter

The eMMC is the heart of the installed OS. It contains the early boot stages, the normal Linux rootfs, factory content and user settings on one physical device. The Linux block layer exposes the ordinary user area as `/dev/mmcblk0`, divided into eight GPT partitions. The total captured user area is exactly **7,820,083,200 bytes** (approximately 7.28 GiB). Each sector in the table is 512 bytes.

| Partition | Name | Start LBA | Sector count | Nominal capacity | Main function |
|---|---|---:|---:|---:|---|
| `mmcblk0p1` | `uboot-spl` | 64 | 128 | 64 KiB | Rockchip ID block and SRAM loader |
| `mmcblk0p2` | `env` | 192 | 16 | 8 KiB | U-Boot environment |
| `mmcblk0p3` | `uboot` | 256 | 768 | 384 KiB | U-Boot executable image |
| `mmcblk0p4` | `splash` | 1,024 | 8,192 | 4 MiB | Boot splash image |
| `mmcblk0p5` | `recoverysplash` | 9,216 | 8,192 | 4 MiB | Update/recovery splash image |
| `mmcblk0p6` | `rootfs` | 17,408 | 1,024,000 | 500 MiB | Factory Linux, kernel, DTBs and services |
| `mmcblk0p7` | `content` | 1,041,408 | 8,388,608 | 4 GiB | Factory content |
| `mmcblk0p8` | `data` | 9,430,016 | 5,843,551 | ~2.79 GiB | User projects and persistent operating-system state |

The names in the GPT are useful descriptors, but partition names are not protection mechanisms. The hardware will still allow certain lower layers to address raw sectors if the appropriate software and permissions are present. That is why the MPFree installer cannot equate a “rootfs only” intent with safety merely because a command line names a partition. It must validate the actual partition table, source image length, intended LBA range and current device identity.

The eMMC also has two dedicated hardware boot areas, exposed in Linux as `/dev/mmcblk0boot0` and `/dev/mmcblk0boot1`. They are not `mmcblk0p0` or `mmcblk0p1`. Both were 4 MiB and entirely zero in the owner’s verified reference capture, which supports the conclusion that the normal first stage is stored in the **user area** at LBA 64. Linux exposed them with `force_ro=1`. Configuration in eMMC’s EXT_CSD can influence hardware boot selection, so an installer targeting other units must still inspect the actual state rather than assume it is identical.

An eMMC device may also contain an RPMB area with authenticated access semantics. The initial whole-disk copy covers the user area and separately captured boot0/boot1, but it does **not** mean that RPMB state, every EXT_CSD setting or any SoC one-time-programmable state has been backed up or is freely restorable. These are different classes of persistent hardware state and are outside the permitted early MPFree installation surface.

### 3.1 Where the kernel actually resides

Many embedded products have a dedicated kernel partition separate from the rootfs. The MPC One does not. Partition `p6` is a 500-MiB ext4 filesystem, and its `/boot` directory contains the zImage and device trees. The first bootloader stages can be intact even if that filesystem is corrupted. A replacement `p6` that contains the right kernel, boot script, DTB and rootfs could *in principle* be started directly by the existing U-Boot.

That fact makes replacing p6 tempting. It would remove the factory Linux userspace from the normal power-on path while preserving the earlier loader. However, **p6 is also the working factory recovery baseline**. Overwriting it is not the same as installing MPFree alongside the factory system. A failed p6 replacement could leave the board dependent on U-Boot Update Mode for recovery, and that exact restoration path has not been rehearsed on the reference device. The correct next milestone is a nonpersistent pre-Linux MPFree boot, not overwriting p6.

### 3.2 Why the small `env` partition deserves special care

U-Boot’s environment is only 8 KiB, but it controls where a much larger system starts. It contains the boot targets and script values used long before Linux can repair a typo. A single environment change could make U-Boot look for a nonexistent file or choose a faulty image. Even if there are default values compiled into U-Boot, whether and how those defaults are selected after corruption must be demonstrated on this board.

For the first independent-boot experiments, changing `boot_targets` or invoking `saveenv` is therefore **not** an acceptable shortcut. A temporary U-Boot RAM-boot mechanism or verified removable-media selection would provide the same information without altering persistent boot configuration. Only after independent recovery has been proven should MPFree add an automatic, redundant boot-selection policy.

### 3.3 The backup we actually have

A read-only stream of the entire eMMC user area was captured to the Linux workstation, together with the two hardware boot areas. The backup was checked against hashes calculated from the reference MPC: GPT/MBR and boot-critical partition regions agreed with the bytes copied to the workstation. The p1, p3–p6 and boot0/boot1 reference contents were also checked during later boot tests. These are strong guarantees of **capture integrity**.

They are not a guarantee that the image is a perfect live snapshot of every writable file. Partitions p7 and p8 were normally mounted read-write and may have changed while the stream was copied. Nor does possession of an accurate 7.82-GB image prove that the board can be restored once its installed loader will not start. A complete backup and a tested independent recovery mechanism are separate deliverables.

The full private image also contains proprietary firmware, factory samples, personal projects, device identifiers and SSH material. It belongs on the owner’s protected workstation or offline backup media, not in the MPFree Git repository. Public documentation can use partition geometry, nonidentifying hashes and independently developed tools without distributing those contents.

