# MPFree broad bootchain round — seven-track strategy and new findings

Date: 2026-11-12. Research branch: research/wide-bootchain-audit-20261111.
This is a public **source-only** research note. All original owner firmware,
eMMC backups, complete Ghidra pseudocode and private integrated ZIPs remain
outside the repository.

## Research protocol for every future iteration

Every integrated round must produce a single owner-private, indexed ZIP with
one question/evidence matrix spanning:

1. **RK3288 BootROM/MaskROM** — ROM recovery entry, USB OTG wiring,
   ID blocks, boot-media order, fuse/secure boot gate.
2. **SPL/DDR/Handoff** — SRAM entry, ROM-provided loader, MMC image validation,
   DDR timings, indirect U-Boot transfer.
3. **U-Boot/Fastboot** — boot-image handling, ROM reboot magic, RAM boot,
   Fastboot permission initialization and dispatch, SDMMC probing.
4. **Memory** — relocated bootloader, download buffer, kernel/FDT/ramdisk,
   first-stage SRAM limits and corruption/overlap safety.
5. **Firmware update 3.6→3.10** — update.img image structure, userspace
   verify/apply, ramfs-update, trustpoints, reboot flags, rollback options.
6. **USB transports** — Fastboot/RockUSB/unknown FF/61/7A selection,
   message-state matching, synthetic error states, isolated fake USB.
7. **Recovery** — owner backup provenance, independent complete readback
   and restoration, original bootchain reversibility.

A question is **answered** only with cited static evidence, upstream
documentation or actual physical results, each carrying the appropriate
confidence label. Each unanswered question must have a specific next probe.
No private proprietary code is copied into public documentation. Complete
owner-private evidence reports remain hash indexed.

## New cross-track conclusions from owner-private 3.10 and 3.6 reports

### RK3288 ROM boot order differs from U-Boot boot_targets

The RK3288 technical reference manual describes BootROM checking external
boot-media ID blocks and falling back to USB OTG. The order depicted in its
flowchart includes eMMC ahead of SDMMC.

The MPC's **saved** U-Boot environment, at the later stage, has
boot_targets="mmc0 mmc1" and bootdelay="0"; its compiled-default candidate
matched all 28 saved variables.

**Additional discovery from the owner-verified embedded U-Boot FDT:**
`mmc0` aliases `dwmmc@ff0f0000`, an active 8-bit, non-removable
controller characteristic of eMMC. `mmc1` aliases
`dwmmc@ff0c0000`, but that controller is explicitly
`status = "disabled"` in the embedded U-Boot FDT.
Thus U-Boot's configured fallback target is not obviously usable,
even though `mmc1` appears in boot_targets. This is static evidence:
runtime board code or overlays might alter the device state.

The SoC BootROM itself also searches eMMC before SDMMC (per RK3288 TRM).
Inserting an SD card therefore cannot be assumed to bypass the stock
first-stage loader. Before relying on SD-card recovery, prove whether
stock U-Boot can dynamically initialize mmc1 and whether ROM can boot
external storage without first selecting eMMC.

Upstream RK3288 source/board information:
- https://www.phytec.de/documents/?title=phycore-rk3288-manual
- https://rockchip.fr/Rockchip%20RK3288%20TRM%20V1.0%20Part%201-System%20and%20System%20Control.pdf

### Firmware 3.10's mode-update is an alias for loader, NOT ROM magic

The factory-versus-3.10 DTB diff for both ACVA variants shows:
- mode-loader = 0x5242C301.
- 3.10 adds mode-update = 0x5242C301.
- mode-bootloader = 0x5242C309 (Fastboot in upstream definitions).
- maskrom/BootROM download magic defined in Rockchip U-Boot:
  BOOT_BROM_DOWNLOAD = 0xEF08A53C.

The Linux syscon reboot-mode driver writes named reboot-mode values to a
syscon register that firmware can examine. Thus, 3.10 mode-update is not a
distinct BootROM request value; the actual update reboot path and physical
ROM availability are still unproven.

Related sources:
- https://github.com/rockchip-linux/u-boot/blob/next-dev/arch/arm/include/asm/arch-rockchip/boot_mode.h
- https://code.googlesource.com/linux/torvalds/linux/+/3c8f504b3a486e4e984ac8dc619eba3afa24cec4/arch/arm/boot/dts/rk3288.dtsi
- https://lists.infradead.org/pipermail/linux-rockchip/2026-September/077438.html

The 2025/2026 upstream USB 0x471/0x472 RAM-boot work is a relevant
independent-recovery lead but does **not** establish an RK3288-compatible
MPC firmware loader:
https://lists.u-boot-project.org/pipermail/u-boot/2025-November/603421.html

### Actual U-Boot Fastboot permission check is visible

The static flash and erase functions at ELF 0x00122C5C and 0x00122BF8
both call the Fastboot permission function at 0x00122834. The function
reads a global state and denies a request when false.

This is evidence of a real write gate, but **not** of its runtime
initialization, an authorized unlock procedure, or a safe storage
write. The Fastboot RAM boot function at 0x001226B4 follows a separate
path through bootm; whether that path shares the write gate remains open.

Modern U-Boot documentation makes clear that Fastboot RAM boot,
getvar and optional flash/erase functionality depend on build configuration;
it cannot replace inspection of this exact vendor binary.
https://docs.u-boot-project.org/en/latest/android/fastboot.html

### The 3.10 executable boot script did not change from 3.6

The exact normalized U-Boot script contents agree between the saved 3.6
rootfs and extracted 3.10 rootfs. It loads zImage and board FDT into
the configured address slots, optionally applies an overlay for az01b,
and invokes bootz.

Version differences therefore need investigation in the kernel, DTB,
userspace updater and trust infrastructure, not assumed boot-script
redesign. The 3.10 rootfs also includes:
- az01-bootloader-sig, absent from the 3.6 backup;
- az0x-loadpin-trustpoints and an early oneshot verity/LoadPin service;
- ramfs-update.squashfs.

These are owner-static presence results, **not evidence** that the
immutable BootROM now requires signatures, that dm-verity is active
at every boot, or that the original OS can be replaced without
additional verification.

### Current USB status is useful but not the end of the investigation

FF/42/03 is Fastboot-compatible, FF/06/05 RockUSB-compatible,
and FF/61/7A uses a custom framed request/response mechanism.
Our synthetic model's 17 tests and defensive policy model's
14 tests passed on the owner workstation; all 12 ARM return
contract tests passed as well. These validate our code, not
the original vendor USB implementation.

Keep two separate models:
- conservative MPFree policy allowlist;
- control-flow approximation that accepts any response identifier
  matching the outstanding request and tracks the unknowns.

Actual vendor-code exercise must be deferred until a hardened outer
virtualization boundary, fake transport with no actual USB path,
and no accessible writable firmware storage are in place.

## Source changes planned for next integrated owner run

- Expand V11 directed Ghidra target selection to additional ROM/SPL
  entry and handoff sites, Fastboot permission and bootm/image
  verification, updater entrypoints.
- Capture bounded ARM machine-code evidence for more SPL and U-Boot
  targets, not only FF/61/7A functions.
- Create private wide-research-matrix.json and .md with **42 or more**
  specific research questions spanning all seven tracks.
- Include bounded per-role Ghidra decompilation/ARM instruction counts,
  direct and computed call information. New facts must be distinguished
  from old evidence and upstream reference material.
- Use synthetic regression tests before rerunning long Ghidra/QEMU
  sessions; keep one private ZIP for the owner to upload.

## Still unsafe to conclude from static-only results

No proven physical RK3288 BootROM enumeration, owner SPL acceptance,
actual eFuse security policy, rootfs/write-gate authorization, runtime
memory relocation boundaries, verified SD-first boot route, or
independently demonstrated full eMMC restoration.

Do not write bootloader partitions, mutate fuses, issue destructive
Fastboot commands or attempt bootloader replacement based on
these source-only results.
