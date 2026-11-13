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


## 4. What Linux mounts, and where changes really persist

After U-Boot starts Linux, the kernel mounts `p6` as its root filesystem. The reference system mounts it **read-only**, as ext4 without a journal. This is a deliberate embedded-system design: the operating system’s base programs and libraries are intended to remain relatively stable, while user data and configuration are stored in separate writable areas.

If that were the whole story, modifying a configuration file under `/etc` might appear impossible, because `/etc` normally lives on the root filesystem. The MPC instead uses **overlayfs**. Overlayfs combines a lower layer, which supplies the original files, with a writable upper layer that stores changes and replacements. A process opening `/etc/some.conf` sees one apparent directory, even though the underlying bytes can come from different partitions.

On the reference MPC, the upper layers for `/etc` and `/var` are stored under `/data/system` on the writable `p8` partition. That means a command which appears to modify `/etc` may be writing persistent eMMC data even while the root ext4 filesystem itself is still mounted read-only. This distinction has repeatedly mattered in MPFree planning because a “read-only root” is not equivalent to a **read-only whole system**.

The important observed mounts are:

| What Linux sees | Backing storage | Persistence |
|---|---|---|
| `/` | `/dev/mmcblk0p6`, ext4 read-only | Changes require special action; baseline intended immutable |
| `/data` | `/dev/mmcblk0p8`, ext4 read-write | Persistent |
| `/media/az01-internal` | Second mount of **the same p8** | Persistent; not separate space |
| `/media/acvs-content` | `/dev/mmcblk0p7` | Persistent factory content |
| `/etc` | Overlay with upperdir `/data/system/etc/overlay` | **Persistent** |
| `/var` | Overlay with upperdir `/data/system/var/overlay` | **Persistent** |
| `/run` and `/tmp` | RAM-backed tmpfs | Lost at reboot |
| Typical USB FAT/exFAT media | Automounted under `/media` | External and frequently `noexec` |

For an OS developer, there are two implications. First, treat the overlay-backed files as genuine persistent configuration, and back up or isolate them before changing services. Second, choose installation paths deliberately. A temporary test that must disappear at power-off should stage its files under a verified RAM-backed path, not under `/data` or an overlay-upper directory. A persistent but reversible MPFree install should normally use an explicit isolated directory on approved removable media or under `/data/mpfree`, with a separate backup and cleanup policy.

### 4.1 Read-only is a property of a mount, not an absolute barrier

Even within a read-only Linux mount, a program with sufficient access can interact with raw devices or ask the kernel to change mount state. Conversely, the absence of a visible mount is not proof that a block device does not exist. Therefore a rigorous no-eMMC-write test does not stop at “we mounted the root read-only.”

The temporary 3.10.0 boot environment used a stronger strategy: the replacement device tree marked the internal eMMC controller `ff0f0000` and the SD controller `ff0c0000` as disabled. The replacement kernel consequently did not bind the corresponding MMC block drivers, and the expected internal `/dev/mmcblk*` devices were absent. This made it impossible for ordinary update programs in the replacement userspace to open those Linux block devices. The test also replaced `/data` with tmpfs, masked updater-related services and prevented another `kexec` handoff.

Those measures were layered deliberately. If a startup service unexpectedly attempted to touch internal data, the expected device nodes were unavailable. If a setting was written under `/etc`, the overlay upperdir pointed into RAM. If an updater attempted to launch yet another kernel with different device-tree settings, that path was not available in the isolated image. Finally, measurements before and after confirmed that the protected boot partitions had not changed.

This is a useful *pattern* for MPFree. It is not a universal physical write-protection guarantee against arbitrary privileged code that deliberately changes device-tree bindings, performs raw register operations or controls other physical peripherals. The guarantee belongs to the **tested image and its enforceable restrictions**, not to any Linux kernel simply because its root filesystem is on USB.

### 4.2 What the directories mean to an installer

Several filesystem locations deserve distinct treatment:

**`/boot`** is part of the rootfs and carries the kernel, DTBs and U-Boot script. A release which replaces these files changes how the next cold boot works. It is boot-critical even though it looks like an ordinary directory.

**`/usr/lib/systemd/system`** contains service definitions. An earlier MPFree experiment installed a persistent SSH unit here by temporarily making p6 writable. That has been documented and checked, but repeating such rootfs modifications is not recommended for the normal installer because the rootfs is small, nearly full and nonjournaled.

**`/etc` and `/var`** contain persistent overlays. A harmless-looking edit to a service configuration can survive a reboot and interact badly with a later factory firmware update. These directories should not become the default MPFree deployment location.

**`/data`** is the main persistent user/system area. A scoped `/data/mpfree` directory could be an appropriate location for an opted-in MPFree application installer, but a future factory update or reset could erase or alter this partition. Essential MPFree releases and user projects should have an independent removable-media or workstation backup.

**`/media/acvs-content`** contains factory assets. Its data should be treated as owner-held content, not scratch space. MPFree may eventually provide local-only tools to import compatible sounds and instruments without redistributing licensed files.

**`/run` and temporary filesystems** are suited to transient helper binaries, logs and handoff state. They disappear after reboot, which is exactly what we want for early reversible experiments.

## 5. The factory Linux distribution and service graph

The factory system is not a tiny monolithic executable. It is a Yocto-built distribution with glibc, BusyBox and other Unix tools, systemd, device-specific shared libraries and several services for board initialization. The main music-production executable eventually runs as a systemd service. Understanding that lifecycle is necessary whether MPFree intends to replace only the application or eventually replace all of Linux.

The key service is `acvs.service`. It invokes `/usr/bin/az01-launch-MPC`, which in turn launches the executable at `/usr/bin/MPC`. The service is configured to restart after failure, with a short restart delay. Therefore killing the user-facing application process does not necessarily release the audio and display hardware for an alternative program; systemd may simply start it again. A controlled handoff needs to stop the service as a unit and verify that the relevant processes and file descriptors have closed.

The application also participates in a wider dependency graph. Other services initialize network MIDI, touch and peripheral firmware, audio support, device communication and USB script handling. Some of these are necessary for hardware to function even if the MPC application is not running; others are specifically part of factory workflows. We must identify that boundary rather than disable every service whose name begins with `az01`.

### 5.1 Two strategies for replacing the factory music application

The **application-replacement strategy** leaves the factory kernel and system services running, then stops the main MPC service and starts MPFree. This is the least intrusive way to develop the DSP, sampler, sequencer, UI and controller handling, because the existing kernel already knows how to initialize the board. It requires a reversible service handoff, a timeout and a supervisor that can restart factory services if MPFree exits or fails its health check.

The **independent-userspace strategy** boots another root filesystem and starts only the services we select. A `kexec` boot is one way to achieve this while leaving the installed U-Boot and eMMC untouched. It gives MPFree control over its own init configuration, device service ordering and application lifecycle without competing with a live factory MPC process. The tradeoff is that we must now supply the necessary userspace libraries and hardware setup ourselves.

The second strategy is closer to the eventual independent MPFree OS. It separates the actual music-production runtime from factory user-space assumptions while still using the proven factory startup path as a temporary launcher.

### 5.2 ARM ABI and binary compatibility

The installed system has 32-bit ARM EABI5 hard-float userspace, not AArch64. A binary’s file format must match the processor and runtime. For Rust, the usual target is `armv7-unknown-linux-gnueabihf`, and the build needs a suitable ARM hard-float linker. Existing MPFree Rust components were cross-compiled for this architecture and have run on the physical device, including rendering and sequencer tests.

For dynamically linked binaries, merely compiling with an ARMv7 target is not sufficient. The executable’s `PT_INTERP` entry names the dynamic loader it expects, and its `DT_NEEDED` entries identify the shared libraries it needs. Individual symbols may require particular glibc versions. The reference libc has been explicitly identified as glibc **2.39**, and the dynamic loader path is `/lib/ld-linux-armhf.so.3`. Some earlier conclusions based only on `GLIBC_*` strings were weaker; later measurements established the actual release. Every new binary should still be checked against the intended target firmware rather than assume library compatibility across all MPC revisions.

A static helper program reduces dependence on factory shared libraries and is attractive for an early launcher. It does not solve everything: device-specific access may depend on kernel modules, udev rules, userspace firmware and graphics/audio libraries. A fully independent MPFree Linux distribution will eventually provide its own versions or properly licensed components for these facilities.

### 5.3 Realtime audio requirements are separate from boot requirements

The factory `PREEMPT_RT` kernel is an important advantage, but an audio engine needs more than a realtime-capable kernel. It must choose appropriate scheduling, CPU affinity, memory-locking behavior, buffer sizes and ALSA device access. The reference command line’s `isolcpus=2-3` suggests an intended partitioning of CPU work, but observed per-process scheduling and affinity must be interpreted separately. The MPC’s main process was seen holding approximately a gigabyte of locked memory in one snapshot, while one service-limit report suggested a much smaller per-process memory-lock limit. That discrepancy remains unresolved and should not be turned into an assumed recipe for MPFree.

The audio engine can initially use the stock kernel’s known ALSA/DRM drivers. The move to a new kernel should be an independent qualification step: measure actual round-trip audio behavior, IRQ routing, XRUNs, CPU scheduling and device initialization. No amount of early bootloader work replaces the need for those realtime tests.

## 6. Hardware devices exposed to Linux

The operating system communicates with the MPC hardware through normal Linux device subsystems. Much of the work required to build a free music environment is therefore about documenting **which kernel interface corresponds to which physical feature** and establishing how the factory application uses it.

### 6.1 Audio and MIDI

The reference system exposes an ALSA card labelled `ACVA` with playback and capture PCM devices, plus an MPC MIDI card. Several raw MIDI ports were enumerated, including Public, Private and MIDI Port interfaces. We also observed that the factory application opens relevant ALSA, MIDI sequencer and raw-MIDI handles during normal operation.

That ownership is why an alternative audio application may fail to open a PCM device while the stock MPC process is running. It is not necessarily evidence of a missing driver or permission problem. The first question is whether the device is currently in use by `acvs.service`; the second is whether MPFree has chosen the right ALSA card, direction and format. Some MIDI and controller interfaces have different behavior depending on whether the factory process has initialized them. The project’s existing controller and MIDI documents describe those differences in greater detail.

For an independent OS, the goal is to initialize the sound hardware directly and let MPFree own the PCM and MIDI endpoints without needing the factory application to prepare them. That requires confirming codec/mixer controls, routing, buffer format, sample rates, firmware dependencies and safe shutdown behavior. The kernel exposes the basic interfaces, but the full audio hardware initialization contract deserves its own tests.

### 6.2 Display, graphics and touchscreen

The device contains a DSI display driven by the Rockchip display subsystem. The live Linux interface includes DRM/KMS devices and a framebuffer, and the graphics stack exposes Panfrost GPU functionality for the Mali GPU. The observed panel mode is 800 × 1280 in the native DRM orientation; user-facing screen orientation and rendering transforms are separate concerns.

The touchscreen appears as an Ilitek ILI2117 input device, and physical keys appear through another event interface. In the initial survey, touchscreen and GPIO keys were available as `/dev/input/event0` and `/dev/input/event1` respectively. Those event numbers are **enumeration results**, not guaranteed identities across every boot or firmware release. A robust application identifies devices by name, properties and stable paths rather than hard-coding event indices.

A future MPFree compositor or direct framebuffer UI has to manage DRM mastership, modesets, touch coordinates and device ownership. The factory process normally owns key UI resources. In the isolated 3.10.0 boot, we demonstrated an independent on-screen log and a small application without running the factory UI. This is particularly useful evidence: MPFree does not need to wait for a replacement cold bootloader to begin developing its independent graphics environment.

### 6.3 USB host, USB device and removable storage

The board has two relevant DWC2 controller roles. Linux describes `usb@ff540000` as a host controller and `usb@ff580000` as the peripheral/OTG controller. The latter appears as `/sys/class/udc/ff580000.usb`, and a `usb-otg-mux` component exposes selectable roles. The standard USB-A sockets support storage and peripherals through host mode. The rear USB-B interface is the likely connection for the device-side controller used in the desktop update workflow.

These USB paths play different roles throughout the boot sequence. A USB drive inserted into a host port can carry files for factory Linux, including the older autorun script and a removable MPFree rootfs. The device-side USB-B port can carry Fastboot or other protocols when the corresponding low-level loader is active. And the silicon RK3288 has a ROM-level USB recovery facility whose physical entry is still not demonstrated on this MPC. **All three may involve USB, but they are not the same service and do not have the same recovery properties.**

The stock firmware commonly mounts FAT/exFAT USB volumes with `noexec` and related flags. A shell autorun file can still be interpreted by a script runner, but that does not imply a compiled ARM executable can be started directly from the same mount. The development launcher should use a known executable RAM-backed directory or an approved Linux filesystem rather than assume that “being on a USB stick” is sufficient.

### 6.4 Ethernet and networked development

The MPC has a Rockchip GMAC at `0xff290000` and a Realtek RTL8211E Gigabit PHY. Direct Ethernet between the workstation and MPC was used for SSH, file transfer and repeated hardware surveys. A working 1-Gb/s link was measured. Ethernet is an excellent development transport because it keeps command/control separate from the USB-B bootloader service and avoids continually changing USB media for small updates.

The PHY’s reset behavior also became an important part of the alternative-kernel work. After the first `kexec` handoff, the new Linux kernel initially failed to read the PHY’s ID. Later changes to the FDT provided an explicit PHY identifier and correct reset timing. One subsequent network initialization still failed on its first attempt, but a retry successfully established a 1-Gb/s link and SSH. This shows that the exact FDT and **hardware state across a warm kernel handoff** matter even when the driver is present in both kernels.

### 6.5 Other peripheral controllers

A complete open-source music workstation must eventually qualify hardware controls, firmware update requirements for peripheral microcontrollers, LEDs, pads, storage adapters, MIDI routing, power management and audio codec behavior. Some existing services update touch, XMOS or other peripheral firmware under specific conditions. Those services should not run automatically in a test root where their dependencies and target hardware have not been qualified. Equally, an independent OS should not disable an unknown service and then assume a peripheral works without its initialization.

The preferred implementation discipline is to track every hardware interface by controller/FDT node, Linux driver, device file, factory owner, experimental MPFree owner and validated initialization/shutdown procedure. The existing [audio](audio-runtime.md), [controller](controller-protocol.md), [resource-ownership](resource-ownership.md), [network/USB](ethernet-usb.md) and [touchscreen/display](touchscreen-display.md) references provide the starting inventory.


## 7. The practical breakthrough: booting another Linux kernel with `kexec`

One of the most useful discoveries was that the installed kernel supports `kexec`. This is a Linux facility for loading and starting a replacement kernel **without returning through the ordinary firmware boot sequence**. The running kernel allocates memory for the replacement image and its associated data, coordinates shutdown of the current system and transfers control to the new kernel. The new kernel then initializes the hardware as a new instance of Linux.

The difference from an ordinary application launch is substantial. Starting MPFree as a process under factory Linux still shares that kernel, its devices, its rootfs and potentially its service graph. Starting MPFree Linux by `kexec` means that the factory kernel stops running. The new Linux can have its own init system, root filesystem, device configuration, update policy and application startup. It can, for example, start our sampler and UI directly without ever launching `acvs.service` in the new environment.

The difference from cold boot is equally important. A `kexec` launch **does not begin at the processor's BootROM**. To reach it, the MPC has already started the factory first-stage loader, U-Boot and at least enough factory Linux to run a developer command. That is why `kexec` is an ideal bridge toward OS independence but not the final bootloader solution.

The reference 3.6.0 kernel contains `CONFIG_KEXEC_CORE=y`, `CONFIG_KEXEC=y` and `CONFIG_KEXEC_FILE=y`, and it ships the executable at `/usr/sbin/kexec`. The official update mechanism also invokes `kexec` to switch into an update-specific Linux environment. Those facts established that the hardware/software stack was designed to support a kernel handoff. We then went beyond configuration inspection: a completely separate firmware 3.10.0 kernel and userspace were started from removable storage **three times on the actual MPC One**.

### 7.1 What the first temporary boot really changed

The test did not install the 3.10.0 firmware onto the MPC’s eMMC. Instead, its root filesystem and associated files were prepared on the workstation and placed on a USB stick. The active 3.6.0 Linux system loaded the replacement kernel and a deliberately modified board FDT from the removable test environment, then transferred control through `kexec`. From that point on, the 3.10.0 kernel and its systemd services were the running operating system.

The most important part of the preparation was **storage isolation**. A normal 3.10.0 kernel would be capable of detecting internal eMMC and running factory maintenance services that knew how to write it. We did not want a temporary test system to have that ability. The new FDT therefore marked the internal eMMC controller at `ff0f0000` and the SD-card controller at `ff0c0000` as disabled. When the new kernel started, it had no corresponding MMC block devices. A service expecting `/dev/mmcblk0` could not access that block device because the controller driver had not been bound.

The test also replaced `/data` with a tmpfs mount. That was necessary because the normal `/etc` and `/var` overlays obtain their writable upper layers from persistent `/data/system`. Without changing the mount arrangement, an apparently temporary kernel boot could still write to the same long-lived configuration directories used by factory Linux. Moving those writable layers into RAM made such changes disappear at power-off.

Additional service controls disabled or replaced the factory app launcher, firmware update helpers, formatting functions, the USB script runner and other programs that were not required for the test. A second `kexec` was disabled to prevent the temporary environment from starting a different kernel that could re-enable MMC controllers. These measures were not necessary for a kernel simply to execute, but they were necessary to make the test’s **eMMC preservation claim meaningful**.

The actual control flow looked like this:

```text
Normal power-on:
  BootROM → SRAM loader → U-Boot → factory Linux 3.6.0
                                         │
                         host-verified temporary launch
                                         │
                           kexec: kernel + FDT in RAM
                                         │
                                         ▼
                                   Linux 3.10.0
                                         │
                          root filesystem on USB media
                          /data backed by RAM
                          internal eMMC + SD unavailable
                          factory update services inactive
                                         │
                          screen / logs / Ethernet / SSH
                                         │
                             physical power cycle
                                         │
                                         ▼
                             original factory Linux
```

We checked the boot-critical partition hashes and block-layer write counters around the experiment. The protected regions matched the reference baseline afterward, and the factory 3.6.0 installation came back after power cycling. This was repeatable, not a single optimistic observation.

### 7.2 First run: the kernel and userspace work, but peripherals need more care

In the first hardware trial on November 10, 2026, the replacement 3.10.0 userspace completed its startup in roughly **7.7 seconds** after the new kernel began. The intended SSH service reached its listening state and the periodic test log was running. The absence of MMC devices and unchanged partition measurements established that the storage isolation was working.

But the Ethernet interface failed. The Rockchip GMAC driver appeared, while the MDIO bus could not identify the PHY at address 1. On the stock system the interface is backed by a Realtek RTL8211E. The most likely cause involved hardware state carried across the warm `kexec` handoff: factory Linux had shut its Ethernet interface down, affecting the PHY's reset and clocks, and the new kernel attempted device discovery before the PHY was in the state it expected. The exact causal mechanism had not been proven.

The display driver also initialized, but a normal framebuffer console was not available, so the kernel's text output did not simply appear on the MPC screen. This was a useful architectural lesson. The fact that a kernel exposes `fb0` does not mean it includes a framebuffer-console driver; an application that wants to display information may need to render explicitly through the framebuffer or DRM subsystem.

### 7.3 Second run: screen output succeeds and the PHY becomes identifiable

For the second trial, the test environment included an on-screen log/status program. It successfully displayed output on the MPC panel. This demonstrated an independently controlled display path in the replacement Linux image without needing the factory music application.

The FDT was also revised to describe the Ethernet PHY more precisely. Its `compatible` property included `ethernet-phy-id001c.c915`, and the documented GPIO4 B0 reset line used the board's active-low behavior with approximately 10 ms asserted and 30 ms deasserted. The new kernel could now identify the RTL8211E PHY correctly. However, the first attempt to bring the Ethernet MAC fully online still failed with a DMA reset timeout. The PHY was now recognized, but its clock or interface state during initial GMAC setup was not yet correct.

This observation separated two different problems that initially looked like one. A missing PHY on MDIO is not the same failure as a MAC DMA engine timing out after PHY identification. A replacement OS needs to distinguish them and not simply conclude that “Ethernet is broken.”

### 7.4 Third run: Ethernet and SSH work after a controlled retry

The third trial repeated the initial GMAC DMA initialization problem, but the test setup retried `ip link set eth0 up` after the system had been running for a short time. That retry succeeded. The relevant timings were approximately:

| Event | Time after replacement kernel started |
|---|---:|
| Initial network start attempted | ~7.8 seconds |
| First DMA initialization timed out | ~8.0 seconds |
| Retry of interface-up request | ~20.6 seconds |
| Gigabit link reported | ~23.1 seconds |
| DHCP address obtained | ~28.0 seconds |
| Workstation SSH session established | ~32.9 seconds |

The link reported **1 Gb/s full duplex**, and the MDIO bus handled transfers without observed errors in the subsequent test. Screen output remained functional. The device's boot-critical partition hashes matched the original baseline before and after. A power cycle returned to the installed 3.6.0 system.

What actually fixed the first network initialization remains a narrower open question. The retry causes additional PHY and MAC reinitialization, and state observations were consistent with the PHY having been reset again. The study considered clock-output settling and Energy Efficient Ethernet behavior as candidates, but neither explanation was proven. A new MPFree kernel can use the measured retry strategy for early tests while a more deterministic first-initialization sequence is developed.

### 7.5 What these tests established for the future OS

The three trials mean we no longer need to speculate about whether this board can run another Linux distribution. It has already done so, with its own root filesystem, services, display output, networking and SSH. We also learned that an isolated test root can be built without giving itself access to internal MMC storage, and that the factory system returns after power cycling when its boot-critical state remains unchanged.

This is the current **lowest-risk demonstrated path** for developing a wholly independent MPFree Linux kernel and userspace on the real hardware. It does **not** prove U-Boot can select a removable image at cold boot or that the BootROM can execute an owner-supplied recovery loader. Those are separate goals.

For complete test details and tool paths, see [the archived 3.10.0 test-boot record](archive/2026-11/kexec-test-boot.md). It includes the original per-run observations and host preparation workflow.

## 8. USB and the boot process: three very different functions

USB is particularly important because it is both a convenient way to supply an alternative OS image and a possible route for servicing the bootloader itself. But it is easy to confuse different software layers that happen to communicate through USB connectors.

The normal USB-A ports are **host ports**. When Linux runs, it detects connected storage and MIDI devices and mounts media using its ordinary drivers. A USB stick can contain the root filesystem for a `kexec` test, private logs, installer data and MPFree application assets. This does not imply that the silicon BootROM or factory U-Boot will treat every USB stick as a bootable operating system.

The rear USB-B port is used in **device** mode for the factory desktop updater. The Linux device tree exposes the Rockchip DWC2 peripheral controller at `ff580000`, which is the expected underlying device controller. When factory U-Boot enters its Fastboot Update Mode, it can present a USB firmware-transfer interface before Linux starts. This is one reason we are interested in its `download` and `boot` handlers: they may provide a way to transfer a RAM-only kernel over USB-B from the workstation.

Finally, the RK3288 silicon ROM has its own USB loading/recovery behavior, which executes before the installed first-stage loader. This would be the most valuable independent recovery layer if we could reliably enter it on the MPC and execute a source-built SRAM program. **That has not yet been demonstrated on the reference unit**, and Fastboot activity must not be mistaken for it.

### 8.1 What the U-Boot Fastboot code already tells us

The existing U-Boot program registers handlers for `reboot`, `getvar`, `download`, `boot`, `continue`, `flash`, `erase` and manufacturer-defined `oem` commands. The code for `download` collects incoming USB data in RAM, and the `boot` path schedules a completion callback that reaches a U-Boot image execution function after acknowledging the USB request. The image-processing path recognizes legacy U-Boot image structures and FDT/FIT headers. These are concrete implementation findings.

They make a nonpersistent pre-Linux MPFree boot look technically plausible. An operating system could theoretically be prepared on the workstation, delivered to the bootloader's RAM buffer and started before the factory rootfs is mounted. But there are still operational uncertainties: which exact image package and FDT placement the bootloader accepts, what its runtime command state allows, how much RAM is safe to occupy, and whether a failed kernel attempt leaves the factory boot available after a power cycle.

Fastboot also has persistent storage commands. They must be kept separate from the RAM-only objective. Their existence is one reason the normal MPFree installer should not experiment with arbitrary Fastboot sequences against the only known-good reference board.

### 8.2 Recovery depends on which stage failed

The important difference is that U-Boot Fastboot **depends on U-Boot**. If `p6` is damaged but the earlier loaders are intact, a desktop firmware update is a credible way to repair the root filesystem. If `p1` or `p3` is invalid, the Fastboot code may never execute. At that point the recovery mechanism must live below the damaged stage, such as an independently demonstrated silicon-ROM loader.

This gives us a clear hierarchy:

```text
Broken MPFree session:
    → power-cycle if internal storage was unchanged
Broken factory userspace or rootfs:
    → U-Boot Update Mode may remain available (needs rehearsal)
Broken factory U-Boot or first stage:
    → independently qualified silicon ROM recovery required
Changed eMMC hardware boot config or one-time state:
    → board-specific recovery, not covered by disk-image backup alone
```

The factory Update Mode itself is not a completed recovery guarantee merely because its commands appear in the firmware. The full restoration procedure must be exercised on suitable hardware before an installer relies on it.

### 8.3 Why software-only silicon-ROM entry remains an open problem

The reference first-stage image includes a Fastboot-related reset mode written to the RK3288 reboot-mode register. That proves a software-controlled way of selecting at least a **loader maintenance** path. It does not prove the installed loader recognizes a request to return to immutable BootROM USB service.

Public Rockchip descriptions include other boot-mode values and USB SRAM-loading mechanisms, but the corresponding behavior was not demonstrated on this MPC. In addition, the actual board must route USB signals, boot-source selection and any required power states in a way that permits entry without damaging the existing system. The safe progression is to establish a genuine ROM USB state, run a minimal independently built SRAM program with no eMMC writes, and then prove full eMMC read-back against the owner-held backup.

That is a legitimate operating-system engineering goal, but **not** a dependency for the immediate `kexec` development path. MPFree can keep progressing while the independent recovery mechanism is qualified separately.


## 9. How official firmware updates actually work

Akai's firmware update system is relevant to an independent operating system for two reasons. It shows that the manufacturer already designed a kernel-to-kernel maintenance transition, and it defines which internal storage regions may change when a factory update is installed.

The reference rootfs includes `/usr/sbin/az01-update` and `/opt/ramfs-update.squashfs`. The latter is a compact Linux userspace intended to run in RAM. It contains a separate init program and enough utilities to verify an update image, locate target storage and install partition contents. The update library `libaz01-prog.so.1.3` includes code consistent with invoking `kexec` to start that RAM environment. One observed command shape loads a kernel from `/boot/zImage` and supplies the updater squashfs as a RAM disk.

The important conceptual step is that the manufacturer does **not** need to update the live root filesystem while its full ordinary userspace remains active. Instead, normal Linux prepares a separate small operating environment, switches to it and lets that environment perform storage maintenance. The normal music-production process and most factory services are no longer running while the updater writes its target partitions.

The updater understands image metadata, board compatibility and individual partition records. It has logic associated with checking image integrity, locating partitions by name, writing compressed contents, updating bootloader regions when required and optionally erasing user data. That means it is unsafe to describe a firmware update as “just copying a new MPC executable.” Depending on the image, the update can affect the Linux rootfs and potentially the earlier boot components.

### 9.1 A firmware update image is not a raw filesystem image

The MPC firmware download is a container that describes what is to be installed. Different generations and device families have used different container layouts. Older inMusic formats can use an FDT/FIT-style container with nodes describing payloads and their checksums. The 3.10.0 Gen 1 image examined for this project used an `AZ01` container. Its header identifies compatible boards, firmware metadata and partition records, followed by compressed payload bytes.

In the studied 3.10.0 case the image contained a rootfs partition record with compressed data. The image format is important for planning a **comparison** between firmware versions, but it is not a general-purpose filesystem and should not be written directly to `/dev/mmcblk0`. The correct interpretation depends on the container's record structure, verification rules and expected installation backend.

The project has host-side tools to examine update images without contacting the MPC. These can report the container structure, extract images into a private work directory and compare the resulting kernel, DTBs, services and userspace files. Such comparison is a good first stage before any official upgrade, especially now that MPFree depends on features such as `kexec` and official developer access.

### 9.2 Two maintenance entry paths

There are two distinct routes into maintenance:

**The USB/network update route** begins in ordinary running Linux, verifies an update and starts a RAM updater kernel via `kexec`. This explains why `kexec` was already supported by the installed kernel and why the facility is not an exotic addition made solely for MPFree.

**The desktop update route** enters U-Boot's pre-Linux Fastboot mode. That path can run without loading the normal Linux rootfs, assuming the installed early loaders remain healthy. It is a plausible recovery mechanism if a factory kernel or p6 rootfs is broken; it has not yet been demonstrated as a complete restoration procedure on this reference MPC.

A related factory-reset operation can use the RAM maintenance environment with an instruction to erase user data. This is another reason MPFree should not place its only copy of projects or release files under `/data`.

### 9.3 The relationship to factory updates and persistence

A firmware update that rewrites `p6` will likely remove any custom systemd unit installed directly in that rootfs, including the earlier persistent MPFree SSH service. Data stored on `p8` may survive, and so may old overlay configuration. Alternatively, the updater includes pathways that can erase data in selected workflows. Both possibilities matter: MPFree cannot assume that its binary survives every factory update, and it cannot assume that persistent overlay state will always be fresh enough for a new kernel/userspace version.

This is why the first real MPFree installer should store its releases in a clearly versioned location that can be removed without destroying the factory rootfs. It should also maintain an exact firmware compatibility record so a developer knows when the installed kernel, C library, device tree or service layout has changed. A successful test on 3.6.0 is evidence for 3.6.0, not a universal guarantee across every future Akai firmware.

## 10. MPC OS 3.10.1 changes the developer-access story

In November 2026, MPC OS 3.10.1 introduced a more explicit set of developer-oriented features. This matters because earlier MPFree work needed a special USB autorun entry and a separately installed SSH service to obtain comfortable command-line access. The official firmware now provides a Developer Panel with terminal and SSH functionality on supported standalone devices, along with an official SDK for external instrument and effect plugins.

The distinction is worth emphasizing. A Linux terminal and SSH service are **access to a running factory system**. The plugin SDK is a supported interface for extending the factory music application. Neither is, by itself, a documented independent bootloader for replacing the complete operating system. Nonetheless, official developer access is a useful step: it reduces the amount of custom setup needed to inspect the board, run diagnostics, transfer a compiled binary or initiate a deliberately controlled development session.

### 10.1 What the official developer facilities provide

The 3.10.1 enhancement controls appear under **Menu → Preferences → Enhancements** on supported devices. The Developer Panel includes Linux terminal access, system logs, service and resource views, and related diagnostics. Akai's release notes say developer mode itself is entered by holding SHIFT + MENU and choosing Developer; turning it on also starts the SSH server, and it adds a script runner that runs `autorun.sh` from any drive while developer mode is on. This means an MPFree developer should no longer assume that USB `autorun.sh` is the only plausible way to reach a shell on a current MPC One.

The same release introduces official external-plugin support. The [inMusic MPC Plugin SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk) provides tooling for building ARM32 and ARM64 instrument/effect plugins and packaging them for installation through the factory host's external-plugin feature. For the Gen 1 MPC One the relevant architecture is ARM32. The SDK and release documentation distinguish the standalone device family from desktop software and explicitly require a supported firmware version and enabled enhancement.

Other optional 3.10.1 facilities include mouse input, screenshot capture, remote-screen/audio access, expanded audio/plugin track limits and additional performance controls. These features matter to general development but do not change the hardware boot sequence on their own.

Because the Developer Panel can expose high-access administration, it must be configured with the same care as any embedded Linux SSH service. A developer should confirm its actual authentication behavior and network exposure on the physical unit. An isolated direct Ethernet connection remains preferable to leaving a highly privileged interface accessible on an untrusted network. The exact server port, credentials and persistence on this reference unit have **not** been established in a post-upgrade hardware test.

### 10.2 What we actually established about firmware 3.10.1

Before updating the reference MPC, the 3.10.1 Gen 1 update image was compared offline with the prior tested firmware materials. The comparison found that several important components were unchanged: the examined kernel image, board DTBs, U-Boot boot script, update launcher and supporting update library. The embedded kernel configuration still contained the relevant `kexec` options, including `CONFIG_KEXEC_CORE`, `CONFIG_KEXEC` and `CONFIG_KEXEC_FILE`.

Those are meaningful results. They suggest that the 3.10.1 developers did not remove the kernel feature we have used for reversible alternative-Linux boots. An unchanged board FDT and boot script also reduce the likelihood of an unrelated difference in those specific files disrupting the known handoff. The compared update RAM environment differed only in version/timestamp information, and examined legacy SSH/autorun-related components were not removed.

However, one component named `az01-signed-fs` **did change materially**. The study reported a change in executable code size and many differing bytes. We do not yet have an established account of the changed component's runtime behavior. It would be irresponsible to conclude either that the change makes owner-controlled kernels unusable or that it is completely irrelevant without understanding its integration points and checking the resulting system.

The important engineering conclusion is therefore more precise than “3.10.1 is safe” or “3.10.1 prevents custom operating systems.” The examined kernel and boot-script surfaces preserved the features we care about, but the actual post-installation behavior of the reference MPC has **not** been measured, and the changed signed-filesystem component still needs qualification. Neither generic Linux kernel configuration nor a byte-identical updater library describes every policy a device might enforce at runtime.

### 10.3 What the offline comparison cannot prove

An update image is a description of an installation, not a complete physical measurement of what the MPC will look like afterward. It does not demonstrate the U-Boot environment the updated unit will retain, the actual eMMC configuration, the contents of every early bootloader region after an update, or which runtime services will be enabled by default. It does not prove that an arbitrary Linux image can start through Fastboot or silicon ROM. And it does not prove that the updated Linux SSH service behaves exactly as the earlier experimental service.

The fact that the official firmware now supports development is encouraging, but **we should verify it, not assume it extends into every boot layer**. A planned expansion of OS-level access in public discussions is a direction of development rather than an installer interface we can ship against today. Our independent boot design must remain functional on the basis of capabilities we have measured.

The focused [3.10.1 compatibility page](firmware-3.10.1-compatibility.md) summarizes the comparison and the remaining upgrade gate. The [stick test runbook](firmware-3.10.1-stick-test.md) tries 3.10.1 by kexec from the USB stick, as 3.10.0 was, before any install. Official information is available from [Akai’s firmware download page](https://www.akaipro.com/downloads-and-support/downloads/firmware/mpc/), [Akai release notes](https://www.akaipro.com/mpc-release-notes/) and [the inMusic SDK](https://github.com/inmusic-software/mpc-plugin-builder-sdk). The November 12, 2026 [Sonic State report](https://sonicstate.com/news/2026/11/12/adc26-akais-new-mpc-os-opens-up-all-models/) also describes the developer-access features and the manufacturer's broader direction.

## 11. The lowest-level access we reliably have today

This question has a surprisingly subtle answer because “access” can mean the ability to observe a component, invoke a service that component exposes, or independently execute owner-provided code **at that stage**.

We can observe the early loader's image, understand where it resides, read U-Boot’s persistent environment and see that U-Boot provides MMC, filesystem and Fastboot functions. That is solid architectural knowledge about code which runs before Linux. But the **earliest stage where we have already started a different Linux kernel on the physical device** is still the factory Linux `kexec` interface. Its no-eMMC-write test behavior was established through repeated runs and before/after measurements.

The **earliest pre-Linux facility known to exist** is U-Boot itself. It has a recovery/USB transport and kernel-execution functions. Once we qualify a RAM-loaded or removable-media image accepted by that installed U-Boot, it may become our earliest *proven* owner-kernel execution stage without modifying boot partitions.

The **earliest potentially factory-independent stage** is the RK3288 silicon BootROM. It is architecturally below the installed first-stage loader, but we have not yet established a reliable software entry into genuine ROM service or an owner-built SRAM loader running through it. It should therefore be treated as a recovery-development objective rather than an already available installer backend.

For MPFree’s requirement that the original system remain recoverable, the present operational answer is: **use the verified `kexec` approach with internal storage disabled for kernel development; use U-Boot only for read-only characterization until a nonpersistent kernel boot has been demonstrated; and leave early eMMC boot partitions unchanged until independent recovery is established**.


## 12. Paths from today's system to a truly independent MPFree OS

There is no need to jump directly from running a Rust application under factory Linux to replacing the first-stage loader. The available architecture permits a gradual sequence in which each milestone yields a more independent operating environment and protects the existing system until the next recovery method has been proven.

### 12.1 Stage A: MPFree as the only music application on the factory kernel

The simplest useful environment keeps the installed kernel and its proven hardware drivers but starts MPFree instead of `/usr/bin/MPC`. There are two ways to test this without making it the permanent startup mode: run on a deliberately isolated Linux image where factory `acvs.service` never starts, or perform a controlled handoff from the running factory service under the supervision of an independent watchdog.

The latter requires careful state management. `acvs.service` has restart behavior; a direct process kill may immediately start another MPC instance and reacquire ALSA or DRM devices. A proper handoff stops the service through systemd, checks that the relevant process and device descriptors have gone away, starts MPFree and confirms its UI/audio health. If MPFree exits or fails to initialize, the supervisor brings back the normal factory service. None of this needs changes to U-Boot.

This stage is valuable for hardware development and DSP performance tuning. It does not satisfy the goal of a complete independent Linux OS, because MPFree still depends on the factory kernel, shared libraries and service layout.

### 12.2 Stage B: an MPFree kernel and rootfs started by `kexec`

This is the best-supported next milestone. We have already proven the technique with the 3.10.0 test environment. Replace the temporary Akai userspace image with a **reproducible MPFree root filesystem** and, in time, a purpose-built kernel using the correct RK3288/az01 configuration. The first boot should retain the existing no-eMMC-access constraints, because driver initialization and userspace correctness can be tested without internal storage being accessible.

The rootfs can contain just the services MPFree needs: a deliberately configured init environment, device setup, audio/MIDI handling, the graphical shell, logging and a debugging interface on direct Ethernet. It need not launch the factory MPC program at all. A power cycle still selects the unchanged factory rootfs, making iteration relatively forgiving.

This stage removes dependence on the **factory userspace after the kernel handoff**, but it still needs factory Linux to initiate that handoff. The next stage addresses that last dependency.

### 12.3 Stage C: kernel selection inside factory U-Boot

The installed U-Boot understands filesystem loading, standard boot scripts and ARM kernel execution. This provides two plausible ways of selecting MPFree before factory Linux starts.

One is to place a compatible kernel, device tree and rootfs on a removable SD or other medium that U-Boot can enumerate. We would then need a verified method of selecting that medium while the factory p6 remains healthy. U-Boot’s `boot_targets=mmc0 mmc1` is a starting point for understanding the scanner, not proof of an existing user-facing alternative-boot key or a preferred-SD configuration. A robust design should prefer **temporary** selection for initial tests, rather than persistent changes to the environment partition.

The other is to use the installed U-Boot’s Fastboot `download` and `boot` functions as a **RAM-only pre-Linux kernel launch path**. This is particularly attractive because a workstation could supply a fresh MPFree build without rewriting storage. It also makes the fallback easy to reason about: if boot-critical state never changed, an ordinary cold boot should still use the existing factory files. However, that reasoning becomes a proven guarantee only after actual image acceptance, memory placement, FDT handoff and fallback have been demonstrated on this hardware.

Achieving either nonpersistent U-Boot boot would meet an important milestone: **MPFree Linux starts without factory Linux ever running**, while the factory first-stage and U-Boot remain intact. That is already a substantial and useful form of OS independence.

### 12.4 Stage D: a persistent but recoverable dual-boot policy

Once nonpersistent U-Boot boot and independent recovery are proven, MPFree can introduce a persistent selection mechanism. Its main responsibility would not be drawing a boot menu; it would be guaranteeing that the last known-good system remains reachable.

An appropriate policy should distinguish the factory entry, the currently confirmed MPFree release, and a candidate release awaiting its first successful boot. A staged candidate should be verified by hash and release signature **before** it becomes selectable. The boot manager should record that it is trying the candidate, let the new system prove that key services started, and mark it confirmed only after a health check. If the candidate fails repeatedly or the removable medium disappears, selection should return automatically to the last known-good entry.

Where this state is stored is a safety-critical choice. It might eventually use a bootloader-supported environment mechanism or a small separate state area, but no persistent boot-selection format should be chosen merely for convenience while the recovery path is unproven. Power loss during any update must not destroy the only known-good boot state. This is the real meaning of a transactional OS installer.

### 12.5 Stage E: independent first-stage loading and recovery

Replacing the Rockchip first-stage loader and U-Boot with owner-built equivalents would remove another dependency on factory software. It would also increase responsibility for DRAM initialization, early storage access, board-specific clocks and boot selection. That work is technically credible on RK3288, but it is not needed to validate the rest of MPFree Linux.

Before attempting it on the reference MPC, the project needs to demonstrate a reliable route into genuine silicon-ROM loading, execution of a small independently built SRAM program, adequate board-specific DRAM/USB/eMMC support, and **full read-back of the installed eMMC** using that independent path. A separate, carefully approved restoration rehearsal would then make early bootloader changes genuinely recoverable. Without that proof, using an accurately backed-up p1 image to overwrite a still-working p1 is an avoidable risk.

The important engineering principle is to **gain a new working entry path before retiring an old one**, rather than hoping an untested replacement will boot.

## 13. What an MPFree installer should actually do

The ideal end-user experience is simple: prepare an MPFree release, connect approved media, confirm the board, install the release and choose which system to start. The implementation must be substantially more careful than that description suggests, because an ordinary Linux program has access to more storage and hardware than it should be allowed to modify during experimentation.

A good design separates the **host-side builder**, **device-side installer**, **boot selector** and **recovery tool**. All four work together, but they are not the same executable and need not be delivered at the same time.

### 13.1 The host is where we prepare and verify the image

The Linux workstation should compile MPFree for the exact ARMv7 hard-float target, build the kernel and FDT, assemble a root filesystem and create a release manifest. The manifest should state which MPC hardware profile and boot method the release supports, which kernel and device tree are bundled, where the rootfs lives, what eMMC access is expected and how the device returns to a known-good state.

The host can verify file hashes, check ELF metadata, inspect partition geometry and confirm that an image fits its intended media. It should also retain the private per-unit eMMC backup and recovery logs outside GitHub. This division is useful because a workstation has substantially more storage and computing power than the MPC and can reject a bad image **before it ever reaches the device**.

A proposed manifest for the currently proven kind of test might look like:

```yaml
schema: 1
project: mpfree
target:
  model: akai-mpc-one-gen1
  compatible:
    - inmusic,acva
    - inmusic,az01
    - rockchip,rk3288
  abi: armv7-eabihf
boot:
  method: kexec-test
  kernel: images/zImage
  device_tree: images/rk3288-az01-acva.dtb
  rootfs: usb-ext4
  disable_internal_mmc: true
  persistent_emmc_writes: false
integrity:
  kernel_sha256: "<digest>"
  dtb_sha256: "<digest>"
  rootfs_sha256: "<digest>"
rollback:
  factory_boot_unchanged: true
  method: cold-power-cycle
```

This format is an **MPFree design proposal**. It is not a structure understood by the current U-Boot or the official firmware updater. Hashes can detect corruption of a release, while an MPFree-maintained release-signing key can establish that a release was produced by a trusted builder. Those functions should not be confused.

### 13.2 The device agent is deliberately conservative

When running on the MPC, the device-side agent should first identify the hardware instead of assuming every MPC model is equivalent. It can inspect device-tree `compatible` strings, kernel identity, partition layout and firmware-version markers. Its compatibility policy should explicitly recognize the tested board and refuse unknown combinations. It should never guess that a Gen 2 model uses the same processor, storage or bootloader as this Gen 1 unit.

Next it should verify that required backup and recovery conditions have been met. For a temporary `kexec` experiment, that may mean checking the owner-held backup manifest, confirming unchanged boot-critical hashes and ensuring the new FDT disables internal MMC. For a persistent installation, it also means validating writable-data backup and rollback. For boot-critical changes, it must require an independently proven restoration path—not simply a file named `mmcblk0.img`.

The agent then stages the release to an approved destination, rechecks the staged bytes and asks for explicit approval before activating a test. Failure at any point should leave the stock boot path unchanged and produce a meaningful report. It should never auto-format an unexpected USB drive because it happens to be plugged in.

### 13.3 A small state machine is better than a long installer script

A robust installer has distinct transitions, not an unstructured series of privileged commands:

```text
Identify exact MPC board and firmware
      │
      ▼
Read boot/storage state and compatibility profile
      │
      ▼
Verify backup and recovery qualification
      │
      ▼
Verify release, media and power conditions
      │
      ▼
Stage immutable candidate image
      │
      ▼
Re-read candidate bytes and verify hashes
      │
      ▼
Owner approves qualified activation method
      │
      ▼
Temporary boot / trial selection
      │
      ▼
Independent health check
      ├──────── failure ────────► retain factory default
      │
      └──────── success ────────► confirm supported release
```

The state-machine approach makes failures easier to reason about. If release verification fails, no boot selection should be changed. If staging fails because the USB device was removed, the incomplete release should be ignored. If the new system fails to initialize audio or display, its health check must not promote it to the default. If power is lost during a trial, the next boot must still have an unambiguous known-good selection.

### 13.4 Keep application installation separate from bootloader installation

A versioned directory of MPFree binaries under approved removable media or `/data/mpfree` is a different kind of operation from writing `p2` or `p6`. The former is an application installation with a manageable cleanup procedure. The latter can change which operating system runs at power-on.

Early releases should avoid modifying the factory rootfs, U-Boot environment and GPT. They can provide a developer-facing launcher that starts a temporary kernel or a controlled userspace environment. A future independent boot selector can be introduced only after a nonpersistent pre-Linux path and its restoration behavior have been demonstrated.

### 13.5 Proprietary sounds and user data should remain separate

The MPFree OS should be redistributable without Akai firmware images or licensed instruments. A user may legitimately own samples, kits and project files on their MPC; a local importer can extract supported data from the user’s own device or backup and place it in MPFree’s private storage. That is conceptually different from distributing the factory assets inside an MPFree image.

The installer should never use factory content partitions as scratch space, and it should not erase p8 while staging MPFree. Its removal procedure should distinguish application files, imported personal content and user-created MPFree projects so that uninstalling an experimental boot manager never means losing music.

## 14. What “safe recovery” means at each failure level

The phrase “we have a backup” describes only one part of recovery. A full recovery promise needs to answer: **what still runs when the system is broken, what device or loader can reach the affected storage, and how do we know that path can restore the exact bytes?**

If a temporary MPFree `kexec` session hangs while internal MMC was unavailable, the normal power-on sequence is still intact. This is the kind of failure that the reference tests actually exercised: power cycling returned to factory 3.6.0 and the protected partition hashes stayed unchanged. That is our strongest present recovery result.

If a normal MPFree application crashes while factory Linux is running, the recovery action can be a service restart or power cycle, provided the application did not change persistent configuration. A supervised service handoff must prove that its cleanup and restart logic really returns audio, display and input devices to the factory environment. Stopping an application is not automatically equivalent to restoring every hardware state it changed.

If the factory `p6` rootfs becomes unusable, the earlier bootloader stages may still load. U-Boot’s Fastboot Update Mode could then accept an official firmware installation without needing p6 to boot. The architecture supports this expectation, but the exact full restoration flow remains to be demonstrated on a suitable fixture.

If U-Boot’s own `p3` image or the first-stage `p1` becomes invalid, its Fastboot command may never become available. Recovery would need a mechanism below those partitions, such as a genuinely independent silicon-ROM SRAM loader. That path remains unproven on the reference MPC. This is why the initial installer must not write p1/p3, even when a full backup exists.

Finally, some persistent hardware state is not represented by the eMMC user-area image: eMMC boot configuration, RPMB state, and possible one-time security provisioning. These need their own hardware-aware understanding. No software installer should casually alter them as part of ordinary MPFree boot development.

### 14.1 A practical qualification ladder

| Level | What MPFree is allowed to do | What has to be true |
|---|---|---|
| **G0: observation** | Read-only hardware surveys and firmware/image comparison | Correct board identified |
| **G1: temporary kernel** | RAM-based isolated `kexec`, removable rootfs, factory power-cycle return | No-eMMC-write design and measurable integrity |
| **G2: persistent application** | Isolated files on approved removable storage or scoped `/data/mpfree` | Backup, service rollback and compatibility tested |
| **G3: direct pre-Linux test** | Qualified U-Boot RAM/removable boot with no critical storage changes | Real image acceptance, safe memory layout and fallback demonstrated |
| **G4: boot-critical install** | Persistent boot selection, p6 replacement, or first-stage changes | Independent recovery and restoration rehearsal already complete |

The system should refuse an operation that requires a gate the device has not passed. A convincing synthetic USB test, an offline bootloader code path, a successful QEMU image boot or the presence of `fastboot` in a command table is not a substitute for the corresponding actual-hardware proof.

## 15. The next engineering milestones, in order

The first immediate task is a **reproducible MPFree Linux test image**. It should use the correct ARMv7 toolchain and board FDT, run an MPFree-controlled init environment, isolate internal MMC and boot through the already demonstrated `kexec` mechanism. It should start MPFree audio/UI services without factory `acvs.service`, retain a diagnostic network path and check touch, pads, MIDI and ALSA behavior systematically. This will establish the independent operating system itself.

The second task is to document **firmware 3.10.1 on actual hardware** under a controlled upgrade gate. Official developer SSH should be tested for configuration, authentication, persistence and service behavior. The changed `az01-signed-fs` component should be understood before the reference unit is treated as fully qualified under that firmware. The system's kernel/DTB/bootloader and partition facts should be re-recorded after any approved update.

The third task is **direct pre-Linux image boot**. Establish actual U-Boot MMC numbering and the full scanning order, then determine a nonpersistent way to select an MPFree image. In parallel, qualify the U-Boot Fastboot RAM `boot` path, including its accepted image format, FDT handling and safe memory limits. The desired result is a working MPFree kernel from removable or RAM media without factory Linux starting and without an eMMC boot selection change.

The fourth task is **independent low-level recovery**. Demonstrate genuine silicon-ROM entry, execution of owner-built SRAM code, board-compatible storage access and complete read-back matched to the saved eMMC backup. Only after a separately tested restore procedure is available should MPFree experiment with persistent U-Boot selection, factory p6 replacement or an owner-built first stage.

The fifth task is the **transactional factory/MPFree boot manager**. It should use the smallest qualified persistent state surface, preserve the factory fallback, verify staged images before boot, mark a release confirmed only after a health check, and recover cleanly from failed releases or power loss. Its design must be grounded in the *actual U-Boot and recovery behavior* of the MPC One rather than a generic embedded-board recipe.

In parallel with those boot milestones, the Rust-based music-production application can continue to mature. Audio DSP, sequencing, sampling, GUI, input handling and external media are largely independent of whether the current kernel was launched from factory U-Boot, `kexec` or a future owner bootloader. Keeping these workstreams separate reduces the chance that bootloader uncertainty stalls the development of the operating system itself.

## 16. What a new MPFree developer should retain

The original MPC One is a conventional 32-bit RK3288 Linux computer with custom music hardware. Its boot chain begins in immutable silicon, loads a small SRAM program from eMMC sector 64, then loads U-Boot from partition 3. U-Boot reads a kernel, board DTB and boot script from the ext4 root filesystem in partition 6, and starts a Yocto-based 6.6 PREEMPT_RT Linux environment. Linux mounts p6 read-only, writes persistent data to p8, and uses p8-backed overlays for `/etc` and `/var`. The music application is launched by `acvs.service` and normally owns ALSA, MIDI, display and input hardware.

A different Linux system has already started on the physical MPC One by `kexec`, using a USB rootfs and a modified device tree that made internal MMC inaccessible. Three tests established the replacement kernel’s operation, on-screen output, functional Ethernet/SSH after a retry, preserved boot-critical storage and restoration of the factory startup through a power cycle.

The installed U-Boot has MMC/filesystem boot functions and a pre-Linux Fastboot interface containing an image-receive and kernel-execution path. These are credible candidates for an independent cold boot without installing a new first stage, but we have not yet completed their on-device kernel-acceptance tests. Genuine silicon-ROM USB recovery and independent first-stage execution are also not yet established.

MPC OS 3.10.1 brings official developer shell/SSH access and an external plugin SDK. Offline comparison indicates that the examined kernel, DTBs, boot script and `kexec` support remain compatible at the file/configuration level; a changed signed-filesystem component needs further qualification. Official developer access improves how we work with a supported running device, but it is not currently a verified replacement-OS installer.

**The safest route to MPFree is therefore incremental:** finish and test our own Linux on the proven reversible `kexec` path; demonstrate a no-write pre-Linux U-Boot boot; establish independent recovery; and only then create a persistent, automatically recoverable boot selection. Each milestone should make the owner less dependent on the factory system without risking the only working copy of it.

---

### Technical references and evidence history

The current detailed low-level reference is [The MPC One bootloader](bootloader.md). The [hardware documentation index](README.md) identifies specialized audio, display, input, networking and storage information. The [Installer and Recovery Contract](installer-recovery-contract.md) defines the practical boundary between temporary testing and persistent changes; the [MPC OS 3.10.1 compatibility page](firmware-3.10.1-compatibility.md) tracks firmware-specific implications.

The original November measurements, including [the storage and boot map](archive/2026-11/boot-map.md), [the 3.6.0 baseline](archive/2026-11/baseline-3.6.0.md), [the three 3.10.0 kernel trials](archive/2026-11/kexec-test-boot.md), [the eMMC backup account](archive/2026-11/emmc-backup.md), [the firmware update study](archive/2026-11/akai-updater-offline.md) and [the independent recovery plan](archive/2026-11/owner-rom-recovery-plan.md), are preserved [in the dated source archive](archive/2026-11/README.md). A more specialized early-loader and USB-program study remains at [docs/dev/research/bootrom.md](../research/bootrom.md).

This reference describes **observed device behavior** where tests were completed and explicitly identifies **image-based findings** and **unverified behaviors** where only stored software or platform documentation is available. Future measurements should update the current explanation, retain the dated source material and state which board/firmware combination was actually tested. No private whole-device image or proprietary factory content is distributed with these documentation files.
