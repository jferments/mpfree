# MPFree sixth integrated owner run — FF/61/7A packet-framing evidence

**Private owner archive:** \`bootchain-wide-integrated-20261112T063013Z-3818692.zip\`.
**Local date:** 2026-11-11 Pacific; ZIP timestamp 2026-11-12 UTC.

This document describes observations from an offline owner-owned firmware audit.
The proprietary pseudocode, binaries, eMMC backup, Ghidra reports, and full
console logs remain private and must not be committed to GitHub.

## Validation and run status

- Full archive CRC passed, **87/87** indexed SHA-256 entries verified.
- Wide SPL/U-Boot Ghidra passed in **55.5 s**.
- Targeted V11 updater analysis passed in **50.2 s**.
- Integrated total **108.8 s**.
- V11 library: **143** decompiled functions, up from 137 in the prior run.
  V11 also decompiled 12 updater entrypoints, 25 directed P1 routines and
  55 directed P3 routines.
- QEMU generic ARM \`virt\` guest booted cloned MPC 3.10 rootfs, completed
  **41/41 serial probe acknowledgments**, and ran synthetic ARM USB
  interface tests **13/13** and endpoint tests **21/21**.
- Both guest updater binary and library hashes matched the source bytes
  analyzed by Ghidra. The real AKAI updater was **never executed**.
- No actual MPC, USB hardware, flash, OTP, or live bootloader was accessed.

## USB protocol identification, kept separate from ROM mode

| Descriptor | Constructor | Callback table | Upstream-compatible family |
| --- | --- | --- | --- |
| FF/42/03 | ELF 0x16ff8 | ELF 0x12a74 | Android Fastboot-compatible |
| FF/61/7A | ELF 0x1b49c | ELF 0x12b48 | **Unknown custom protocol** |
| FF/06/05 | ELF 0x1ca80 | ELF 0x12bd8 | Rockchip RockUSB-compatible |

The private \`usb-protocol-evidence.json\` confirms corresponding Ghidra
constructor xrefs and callback slot recovery. The protocol-family labels
do not prove which hardware boot stage is active, particularly not
immutable silicon MaskROM.

## Strong new static FF/61/7A observations

Decompilation of \`libaz01-prog.so.1.3\` shows a distinct structured binary
transaction protocol:

- Function ELF \`0x1a674\` (framed transfer/response helper) and
  \`0x1a314\` (response matching) both compare message words
  \`0x5aa5fe01\`, \`0x5aa5fd02\`, \`0x5aa5fc03\`,
  \`0x5aa5fb04\`, \`0x5aa5fa05\`, and \`0x5aa5f906\`.
  These are clearly recognized message IDs, not enough to infer their
  English names or whether they belong to ROM versus userland loader.
- Function ELF \`0x1a8d0\` constructs additional message values
  \`0xa55a0101\` and \`0xa55a0202\`. Their precise modes are not yet
  identified.
- The backend context at \`+0x20\` holds a 16-bit counter incremented
  during command construction (e.g. \`0x1a8d0\`, \`0x1a9bc\`,
  \`0x1b35c\`). The response processor compares fields at offsets
  \`+0x04\` and \`+0x10\` to expected values, interprets flags at
  \`+0x14\`, and examines a length at \`+0x16\`.
  Several paths use a 0x18 (24-byte) minimum header.
- Function \`0x1a178\` chunks outgoing payload requests at
  **0x100000 bytes** (1 MiB).
- \`0x1d644\` implements Linux usbfs asynchronous transfers:
  \`0x802c550a\` = \`USBDEVFS_SUBMITURB\` (32-bit ARM \`_IOR('U', 10,
  usbdevfs_urb)\`); \`0x4004550d\` =
  \`USBDEVFS_REAPURBNDELAY\` (pointer-sized \`_IOW('U', 13, void *)\`).
  It uses \`poll()\` to wait and handles EINTR/EAGAIN-like conditions.
  These are **host Linux syscall interfaces**, not device protocol
  opcodes.
- Source verifying Linux USB ioctl names:
  https://github.com/torvalds/linux/blob/master/include/uapi/linux/usbdevice_fs.h

A public search for distinctive \`0x5aa5fe01\` /
\`0xa55a0101\` combinations did not produce a trustworthy published
protocol identification. Retain **UNKNOWN** rather than guessing a
commercial name or assuming BootROM.

Ghidra-decompiler reconstructions can misinfer signedness and structure
layout. Verify candidate offsets against assembly/pcode before making
actual protocol commands, even against a fake USB interface.

## Next integrated offline experiment (committed, not yet owner-run)

Added \`tools/mpc-one/synthesize_custom_usb_framing.py\` and local synthetic
unit tests \`tests/test_usb_custom_framing.py\`.

The original single-run orchestrator now extracts a structured, bounded,
private framing report from the verified V11 library decompilation:
\`usb-ff617a-framing.json\` and \`usb-ff617a-framing.md\`.
These contain function references and decompilation SHA-256 hashes,
known message identifiers, observed header fields, callback slots and
Linux ioctl decodes. It does not put proprietary pseudocode into GitHub.

**Next substantive milestone:** Use independent, instrumented fake
USB endpoints and an additional strong KVM VM isolation layer to observe
the real updater's safe code paths without physical USB, block devices,
network or host-write access. QEMU TCG alone is NOT hardened isolation,
and the current runner does NOT execute the vendor updater.

## Recovery and independent-boot work is still blocked

- No proven physical MPC RK3288 immutable MaskROM enumeration.
- No proven owner-built SRAM loader execution or DDR/eMMC access.
- No independent, verified eMMC restoration channel.
- No validated signedness/fuse-policy readout or RAM-only boot handoff.

These are independent physical research gates. **Do not flash boot
partitions or program OTP** based on this static/VM evidence.
