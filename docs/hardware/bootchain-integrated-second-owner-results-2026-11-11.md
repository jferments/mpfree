# MPFree integrated bootchain research — second owner-run results

Date: 2026-11-11 Pacific (private report timestamp 2026-11-12T050008Z).
The proprietary owner ZIP and Ghidra pseudocode remain private and are NOT
committed to this public repository.

## Provenance and pass/fail

Private owner ZIP: `bootchain-wide-integrated-20261112T050008Z-3773948.zip`.
Its ZIP CRC and all 82 individually SHA-256 indexed entries passed. The
integrated session reported no errors. Broad Ghidra audit completed in 57.6
seconds; V11 targeted Ghidra completed in 45.7 seconds; full run in 105.9
seconds. QEMU ARM32 guest booted and acknowledged **34/34 commands**,
including the final DONE marker. Host vs guest exact SHA256 matches for
`az01-update` and `libaz01-prog.so.1.3` were confirmed.

QEMU here is the generic ARM `virt` board, running a Linux 7.0.14 kernel and
a separate clone of the Akai 3.10.0 rootfs. No original SPL, U-Boot or
BootROM was executed. No physical device passthrough, USB network, or vendor
maintenance binary was executed.

## New evidence: USB interface descriptor selection

The V11 Ghidra output now reconstructs the updater library's USB descriptor
selector at ELF virtual address `0x0001d9fc`. Its decisions inspect
`bDescriptorType=4` (interface), `bInterfaceClass=0xff` (vendor-specific),
and `bInterfaceSubClass/bInterfaceProtocol`:

| USB subclass/protocol | Constructor | Callback table ELF vaddr |
| --- | --- | --- |
| `0x42/0x03` | `0x16ff8` | `0x12a74` |
| `0x61/0x7a` | `0x1b49c` | `0x12b48` |
| `0x06/0x05` | `0x1ca80` | `0x12bd8` |

All three constructors allocate a device context and store the corresponding
callback table pointer at context offset `+0x18`. The generic updater
wrappers subsequently invoke callbacks through that field. Ghidra xrefs
independently connect these table addresses to the constructor code, and the
USB parser calls the corresponding constructors in the matching branches.

This strongly confirms three separate device-interface-specific backend
selection paths. **It does not identify which path is U-Boot Fastboot, ROM
MaskROM, RockUSB loader mode, or another vendor interface.** Those labels
require protocol and hardware-state validation. The scan also recognizes
other vendor IDs, so a table cannot be uniquely labeled based solely on
the Rockchip VID `0x2207`.

The V11 pin list was updated to inspect post-constructor processing
`0x1d924`, descriptor scanning `0x1d9fc`, device-interface enumeration
`0x1d274`, and lower-level USB open/transfer routines.

## Next QEMU experiment (planned, not yet owner-run)

A small **independently written** C test model reproduces only the static
USB interface selection predicates, with 13 synthetic positive, negative,
and malformed-descriptor cases. The single integrated runner now:

1. Cross-compiles the C test as a standalone static ARMv7 binary.
2. Builds a temporary **regular-file** ext4 fixture with the test executable.
3. Exposes that file to the QEMU guest as an additional read-only virtio disk
   (not a host eMMC or USB device).
4. Runs the synthetic test after the guest inventory and records JSON
   case results, console log, and a PASS/FAIL marker.
5. Removes the temporary fixture disk after QEMU exits.

Host-side C/compiler and QEMU serial tests are run as local preflight;
the optional hosted CI workflow was removed because its new jobs failed
before any steps executed (no hosted runner was assigned). The Python
runner syntax and synthetic ARM guest stage still require owner execution.

These synthetic tests do **not** execute vendor USB logic; they validate
our own descriptor-selection model and exercise the guest test/harness
transport. Running the original vendor updater is still deferred pending
additional outer isolation and mocked device interfaces.

## Next important static questions

- What does post-constructor `0x1d924` do with the selected interface,
  and what success/error values drive `0x1d274`?
- What transport framing and endpoint selection correspond to each of
  the three interfaces?
- Which commands are read-only and which would mutate firmware?
- How is cached image verification state 2 (from error -13) interpreted
  across actual image formats, and which write paths are reachable?
- Does U-Boot have a safe nonpersistent RAM/SD boot path with bounded
  memory use? The actual silicon MaskROM and owner-SPL execution paths
  remain independent hardware-proof tasks.

No physical bootloader writes or updates are authorized by this analysis.
