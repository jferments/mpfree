# MPFree: 2026-11-12 second integrated direct-boot audit

Source-only public research summary. Owner-private input:
`bootchain-wide-integrated-20261112T223910Z-26557.zip`.
**Never commit the ZIP, original firmware binaries, eMMC backups, full
Ghidra pseudocode, private raw instruction listings or the 3.10 rootfs.**

## Independent source verification

- ZIP integrity: CRC PASS; 103/103 index entries match SHA-256 and byte
  count; 104 total archive members including the index.
- 108.4 s run: broad Ghidra 56.1s, directed Ghidra 49.0s; both PASS.
- Cloned ARM guest on generic QEMU virt booted. It did NOT execute
  vendor updater, ROM, SPL or U-Boot.
- USB synthetic interfaces 13/13, endpoints 21/21; these are independent
  test models, not real vendor protocol validation.
- Focused 32-question/8-topic direct-boot and broad 42-question/7-track
  static reports PASS. 3.10 small security utility audit PASS.
- Guest P6 compatibility stage INCOMPLETE due unsupported command used
  by the prior analyzer, **not a demonstrated boot failure**.

## New: original U-Boot eight-entry Fastboot dispatch table resolved

P3 U-Boot source SHA-256:
`b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a`.

The SHA-pinned binary references a prefix/callback table at
`0x0012f2b0`, eight entries of 8 bytes. All eight raw callback
pointers have Thumb-mode low bit set. After stripping that bit, they
resolve to the following recognized functions:

| Prefix | Thumb function |
| --- | --- |
| reboot | `0x00122cf8` |
| getvar: | `0x00122d38` |
| download: | `0x00122980` |
| boot | `0x00122964` |
| continue | `0x00122948` |
| flash | `0x00122c5c` |
| erase | `0x00122bf8` |
| oem | `0x00122854` |

This resolves **real command dispatch**, not physical USB enumeration.
The device-side dispatcher only handles a command when its receive
state conditions are satisfied and uses the matched callback. No
Fastboot request was sent on real hardware.

The OEM command comparison, 37-byte token digest, pointer equality and
permission global are previously established. The central RAM-boot
constraint now has stronger support:

- download handler `0x00122980` calls the permission checker
  `0x00122834`;
- flash and erase separately call the same permission checker;
- boot handler `0x00122964` installs the boot-completion callback
  which eventually invokes bootm; it does not directly call the
  permission checker;
- however, **the normal upload path is gated**: boot having no local
  check does not prove a locked session can supply a new RAM image.

The getvar, continue and reboot callbacks have no direct permission
helper call in their decompiled body, but actual dispatcher/lifecycle
conditions and side effects remain separate questions. Reboot and
continue are not read-only operations.

## New: exact U-Boot reboot-mode handling, no MaskROM claim

Function `0x00100fe4` reads/consumes reboot-mode state,
tests its low byte, sets a recovery-mode flag for the values 1 or 9,
and clears the consumed register word. The existing report finds
the reboot-mode MMIO constant `0xff730094` at three
locations. The candidate JC11S boot-counter register
`0xff7300a0`, bootcounter magic `0xb001` and
`0xef08a53c` BootROM download magic are not present as
contiguous word literals in this P3 scan. Computed constants, earlier
stages or runtime device registers may still implement them.

Actual RK3288 BootROM USB entry, owner SRAM loader and whole-eMMC
restoration are **not established** by this result.

## USB host/device transport correlation

The owner-static `libaz01-prog` host selector has
FF/42/03 (Fastboot-compatible, OKAY/FAIL), FF/06/05
(RockUSB-compatible) and FF/61/7A (UNIDENTIFIED framed
vendor protocol). The eight-entry U-Boot table is **device-side
Fastboot**. This supports a possible host/device link, but does
not prove which interface is active on physical MPC hardware in
loader, ROM or normal Linux states. Do not rename FF/61/7A as
RockUSB without evidence.

## P6 original boot script: analyzer failure identified precisely

The cloned 3.10 P6 contains all four expected files
`/boot/boot.scr.uimg`, `/boot/zImage` and both
ACVA DTBs, and modules for `6.18.44-az01-2026-08-10-rt6`.
Its 535-byte boot script hashes differently from the 535-byte
official update extraction:

- cloned guest:
  `edc7e2c871a2359fa9fe07299d6d70b29671222135f46edeb3eb60026fed14cb`
- official 3.10:
  `be713e5e28de35526ea24382c7046a5c4eec3547d295769a44d4d5cad635bdc6`

The **exact reason for the mismatch remains UNKNOWN** because guest
BusyBox v1.36.1 rejects `od -An -v -tx1`:
`od: invalid option -- 'A'`. The next runner now uses
`od -v -b` and a strict octal-byte/offset/terminal parser,
then validates the legacy uImage header+payload CRCs and compares
the executable command text separately from wrapper metadata.
If the new comparison passes, we may conclude identical commands
but a different wrapper; until then, do not make that claim.

## Direct-boot P6 partition and memory constraints

The flash path includes raw block alignment/capacity guards, sparse
RAW/FILL/DONT_CARE/CRC32 chunk handling, and erase alignment/extent
checks. These are *partial* safety measures, not a verified
installer. Partition lookup, extent arithmetic and signed overflow,
download buffer ownership and complete stock-P6 restore remain open.

Configured load address `0x00800800` is only
22.998 MiB below nominal FDT address `0x01f00000`
and 23.998 MiB below kernel address `0x02000000`.
These are address gaps, NOT safe download size limits; runtime
relocation/residency is unverified. A 1.75 GiB advertised maximum
does not establish free RAM.

The saved U-Boot MMC list includes mmc1, but embedded U-Boot FDT
disables three non-eMMC controllers; external SD boot is unproven.

## Firmware 3.10 LoadPin and bootloader signature utilities

Strict SHA-pinned ARM disassembly confirms
`az0x-loadpin-trustpoints` calls ioctl
`0x40044c00`
(LOADPIN_IOC_SET_TRUSTED_VERITY_DIGESTS). The
`az01-bootloader-sig` utility calls RC4
with a 512-byte length, SHA256_Init/Update/Final and 32-byte memcmp.
Neither finding proves normal boot-time enforcement of a stock
U-Boot bootz image or immutable-ROM RSA policy.

The QEMU guest section parser now correctly captures
digit-prefixed `310_trustpoint_unit`; its oneshot
ExecStart is `/usr/sbin/az0x-loadpin-trustpoints`
before local-fs-pre.target. Guest boot was generic Linux on virt,
not the vendor's physical kernel.

## Source-only changes queued for the next owner run

- Fixed guest od invocation to BusyBox-compatible octal byte output.
- Enforced byte offsets, terminal length, byte bounds and uImage CRCs
  before script-command comparison.
- Added Ghidra original-image metadata for the permission global
  and boot/download callback slots; image bytes are **not** runtime RAM.
- Expanded ARM instruction captures for getvar, continue, reboot
  and reboot-mode processing, alongside existing sparse/erase and USB.
- Expanded focused evidence matrix to 40 questions in nine topics,
  adding complete Fastboot handler mapping, PMU reboot mode and host
  USB backend distinction. Existing 42-question seven-track matrix
  continues in parallel.
- Added independent synthetic regression tests for BusyBox od and
  static evidence fail-closed classification.

All changes require **owner workstation preflight** before the next
full integrated Ghidra/QEMU run. No physical Fastboot, USB, MPC
storage or firmware modification takes place in this process.
