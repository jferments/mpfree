# MPFree — direct boot through stock U-Boot, research pivot (2026-11-12)

Source-only report. Private original firmware, eMMC images, Ghidra
pseudocode, machine-code evidence and owner ZIPs are NOT in this repo.
No physical MPC/USB/eMMC is accessed; no vendor executable is run.

## Prioritize these report concepts over USB-only discovery

1. Prove exact OEM Fastboot authorization and permission state.
2. Establish no-write Fastboot RAM boot image formats and memory bounds.
3. Produce an independent ACVA-compatible rootfs/kernel for stock U-Boot.
4. Determine whether SDMMC can be enabled for temporary direct boot.
5. Understand boot-count recovery after warm reboots.
6. Separate 3.10 LoadPin and bootloader-signature checks from bootz.
7. Prove P6-only recovery while continuing independent RK3288 MaskROM research.

## Newly reinterpreted owner-static evidence

Verified original P3 SHA-256:
b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a

| Address | Observed function and fact |
| --- | --- |
| 0x00122854 | OEM command parser uses a 37-byte comparison and writes 1 to a permission pointer on a matching branch |
| 0x0014D690 | Global with a WRITE reference from 0x0012289E and a READ from 0x00122838 |
| 0x00122834 | Fastboot permission helper reads a state pointer and denies when false |
| 0x00122980 | Fastboot download checks permission before accepting a length |
| 0x00122C5C and 0x00122BF8 | Flash and erase each check permission |
| 0x001226B4 | Fastboot boot completion invokes bootm, then resets if control returns |
| 0x00101744 and 0x0010A6A4 | General bootm/image processing, including legacy uImage and FIT handling |

This independently supports the public OEM grant procedure, but the
command has NOT been sent to hardware. Literal-pool pointer equality,
the exact token SHA-256, initialization and persistence remain targets of
the new Ghidra probe. A raw P3 data word is NOT the runtime permission value.

The stock 3.6 and 3.10 normal boot scripts are identical and invoke
bootz on a zImage read from the rootfs P6 filesystem. Fastboot uses a
different bootm path: raw zImage compatibility must not be assumed.
The advertised 0x70000000 Fastboot download maximum is not validated
usable memory; RAM ownership and download bounds need direct analysis.

## External BSP is a reference, not MPC-specific proof

The public openAZ0x az01-mpc machine includes original ACVA but the
maintainer marks MPC hardware validation as pending. Its default kernel
6.18.26-az01-2026-04-30-rt4 and rootfs 430936 KiB are not a drop-in
match for the owner's 3.10 kernel 6.18.44. Kernel modules, DTBs, boot
script and vendor firmware must be matched to the actual device.

References:
- https://github.com/openAZ0x/meta-az0x
- https://github.com/openAZ0x/meta-az0x/blob/scarthgap/conf/machine/az01-mpc.conf
- https://github.com/RedHate/Unbricking-inMusic-Products

P6-only replacement preserves SPL/P2/P3 but is not dual-boot:
stock Linux P6 is overwritten. A previously tested stock rootfs restore
and working vendor Fastboot would be necessary for a reversible install.

## Board-specific recovery concerns

The verified embedded U-Boot FDT maps mmc0 to active, non-removable
8-bit eMMC (ff0f0000), and mmc1 to DISABLED SDMMC (ff0c0000).
Other non-eMMC controllers at ff0d0000 and ff0e0000 are disabled.
Saved boot_targets mmc0 mmc1 does not establish usable SD boot.

OpenAZ0x documents a PMU boot counter at FF7300A0 with upper-half
B001 magic on **JC11S** related hardware. It has NOT been proven
on this older MPC One. The new Ghidra pass scans direct counter
register references, PMU base + offset candidates and B001 magic.
Absence of a literal is not conclusive because code may compute it.
See:
https://github.com/openAZ0x/meta-az0x/blob/scarthgap/docs/az01-hardware-facts.md

No actual owner ROM USB enumeration, independent loader execution,
eFuse security-state readout, or independent whole-eMMC restoration
channel has been demonstrated. Never invalidate SPL/U-Boot to attempt
to force a MaskROM entry before independent recovery is established.

## Implementation and evidence protocol

SHA-pinned Ghidra V11 now collects OEM token/pointer data, the permission
global references, BootROM/reboot/boot-counter constants and bounded ARM
instructions across boot/permission, SPL, update and USB routines.

The integrated run creates a private direct-boot-focus.json and .md
with **25 questions spanning seven topics**, beside the earlier broad
**42-question matrix** and existing Ghidra, QEMU and synthetic USB
evidence. Its focused stage is fail-closed: original Ghidra grant/gate
evidence AND new pointer equality and token SHA must be supported.
The output ZIP remains useful even when a new hypothesis is unproven.

Actual firmware code execution and state-changing Fastboot commands are
deliberately NOT in this round. Future dynamic tests require hardened
isolation, no device passthrough and no mutable vendor storage.
