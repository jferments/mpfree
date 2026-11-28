# MPFree V9 evidence and V10 plan — 2026-11-11

All firmware output remains owner-private and must never be pushed into the public repository.

## V9 source and audit checks

Owner archive: bootchain-wide-20261112T012120Z-v9-3698662.zip. All 16 indexed members passed SHA256; ZIP CRC passed. All 4 Ghidra stages succeeded:
az01-update 61 recognized / 61 decompiled (3 ELF relocation code pointers);
libaz01-prog 472 / 290 (57 relocation code pointers);
SRAM SPL 213 / 100;
U-Boot 1050 / 145. These metrics include PLT stubs and incomplete disassemblies.

## Updater internals (static observations, not yet runtime-validated)

- az01-update 0x0001091c is a high-confidence argc/argv main function found via R_ARM_RELATIVE slot 0x2fe4; calls getopt_long and dispatches to apply-image, USB update, download, and erase-data.
- az01_verify_image (lib 0x00017554) returns 0 when state==1; also returns 0 when state!=0 AND flag bit 1 set. On verifier error -13 it records state=2. az01_write_image at 0x0001e194 calls verify_image(image,2) before compatibility and partition processing. What state 2 means remains unknown. DO NOT assert that every write is protected by signature verification or that this constitutes a confirmed exploit.
- az01_find_devices (0x000145d8) enumerates USB vendor IDs including 0x2207; function 0x000154b8 distinguishes Rockchip VID 0x2207 PID 0x0006 and assigns a protocol state.
- Device context +0x18 points at a backend dispatch structure: open callback offset +0x08, write-bootloader +0x18, write-partition +0x1c, stream +0x20, commit-bootloader +0x2c, reset +0x34. Resolve backend table assignments before inferring actual hardware behavior.
- RockUSB read/write implementation candidates 0x0001c6b8 and 0x0001c208 call a common USB transport with codes 0x80000a14 and 0x00000a15. Function 0x0001c8c8 builds a 31-byte USB command block with little-endian word 0x43425355 (ASCII USBC) and checks a 13-byte response: strongly suggests USB mass-storage BOT framing. Protocol-stage selection remains unproved.
- FDT loader candidate 0x0001c49c dispatches stage transfers using constants 0x440, 0x1400, 0x840, 0x3400. Their units/semantics are not yet verified.
- Updater RAMFS SquashFS has 34 directories, 41 regular files and 198 symlinks. The regular executable /usr/sbin/init exists, but its entrypoint/launching path is still unknown.

## QEMU ARM guest verified

On Ubuntu resolute, generic Linux 7.0.14 ARMv7 zImage built successfully (SHA256 326d095b6770e713c14c4abba7e0fe248f11ebc9bead84c2c994a905f104450b). QEMU virt CPU Cortex-A15 boots the separate clone of the 3.10 rootfs and reaches /bin/sh. Two updater ELF paths exist. /proc is not mounted yet because no init scripts run. Exit of PID1 shell produced normal Linux panic Attempted to kill init; that does not mean boot failure. PCI probing error -75 did not prevent boot.

QEMU virt does not emulate RK3288 BootROM/MaskROM, USB electrical wiring, eFuse, DDR initialization or ROM-independent recovery; no 100% physical recovery claim is supportable.

## V10 exact objectives

1. Resolve the updater main relocation and enumerate its callers and dispatch options.
2. Trace verification state=2 from error -13, format-specific policies and all write call-sites.
3. Find data/function pointer tables implementing backend +0x18 through +0x34 and their selectors.
4. Trace RockUSB command-block framing and USB vendor/product detection versus Fastboot.
5. Trace FDT loader handoff and distinguish ROM versus USB loader state.
6. Continue SPL, SD, U-Boot fastboot/boot-ROM control flow without touching device hardware.

Code: tools/ghidra/MPFreeDirectedUpdaterV10.py, tools/mpc-one/run_boot_updater_v10.py. Both work on verified owner backups and imported disposable Ghidra ELF projects. Owner P1/P3 Ghidra projects are opened read-only. V10 emits one private ZIP.

Run after fetch/switch in mpfree-wide-audit-src:
    python3 "$HOME/mpfree-work/mpfree-wide-audit-src/tools/mpc-one/run_boot_updater_v10.py" --prior-v9 "$HOME/mpfree-work/bootchain-wide-20261112T012120Z-v9-3698662.zip"

Next VM boot with extra no-persistence snapshot:
    python3 "$HOME/mpfree-work/mpfree-wide-audit-src/tools/mpc-one/arm_qemu_lab.py" --launch-ephemeral

Inside QEMU guest shell, before any inspection:
    mount -t proc proc /proc
    mount -t sysfs sysfs /sys
    cat /proc/cmdline
    cat /proc/cpuinfo
    cat /proc/mounts

Do not execute Akai updater or maintenance binaries; start with MPFree's own parsers and synthetic images on virtual storage, ideally inside an additional outer VM. Terminate QEMU from the host rather than exiting PID1. VM overlay rollback is not proof of hardware restore.

Physical bootloader installation remains blocked pending independently confirmed silicon recovery, verified physical eMMC geometry and backups, full bootloader readback, power-loss rollback demonstrations, and a recovery path unaffected by official firmware disabling kexec.
