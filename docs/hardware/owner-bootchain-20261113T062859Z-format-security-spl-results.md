# MPFree owner integrated audit — 2026-11-13T06:28:59Z

Private source: `bootchain-wide-integrated-20261113T062859Z-364945.zip`.
This public note contains only derived, limited facts and our source code.
Never commit the private ZIP, proprietary firmware, rootfs, full ARM
instruction dumps, or Ghidra pseudocode.

## Reproducibility and safety

- CRC: pass. 116 ZIP members total (115 indexed, plus ZIP_INDEX.json),
  all 115 checked SHA-256 digests and recorded byte sizes match; no extras.
- 19/19 integrated stages passed. Broad Ghidra 53.2 seconds,
  directed Ghidra 48.2 seconds; 104.6 seconds total.
- Independent synthetic USB suites: 13 interface cases and
  21 endpoint cases passed. The QEMU guest booted generic ARM-virt
  cloned stock userspace, not original U-Boot or the physical RK3288.
- Seven-track question matrix: 59 questions; direct-boot focus:
  42 questions, ten topics. All existing static evidence stages passed.
- No vendor executable, original BootROM, physical USB, live ADC,
  real Fastboot, eMMC storage, OTP or security fuse was touched.

## USB backend callback evidence (now crosschecked)

All three SHA-pinned library candidate callback tables, their
constructor table references, and Ghidra-resolved method addresses
passed the new independent source-address checks. The Ghidra load
bias is 0x10000.

| Interface | Write bootloader | Write partition | Stream partition | Commit bootloader | Reset |
| --- | --- | --- | --- | --- | --- |
| FF/42/03 Fastboot-compatible | yes | yes | yes | yes | yes |
| FF/61/7A unknown vendor protocol | yes | yes | yes | **no slot** | yes |
| FF/06/05 RockUSB-compatible | yes | yes | **no slot** | **no slot** | yes |

These are table slots, not independently tested silicon capabilities
or proof of a working MaskROM recovery transport. A null slot can
also be bypassed by an entirely different vendor protocol implementation.

## SPL boot sources: named media table and unresolved runtime selection

Original verified SPL SHA:
6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f.

- The five order slots at FF709C70 are all 0xDEADBEEF in original
  image bytes. FF705342 writes a freshly discovered boot source to
  the first slot, so the saved table is NOT runtime priority.
- The first three mapping IDs are 1→MMC1, 2→MMC2, 3→MMC2_2.
  The fourth pair uses ID 0xDEADBEEF and the now-dereferenced
  literal **"unknown boot device"**. This is a fallback label,
  not a fourth confirmed controller.
- The source resolver at FF704120 returns code 1 or code 8 in the
  original decompilation, choosing 8 only after a helper yields
  0x35. Physical meaning of code 8 is unproven; no SD/USB mapping
  should be inferred from these numeric values.
- The first indirect SPL call target FF705350 is source-confirmed.
  The second context callback at FF70A250 reads zero in the saved
  image; its initializer and actual runtime destination remain
  **unresolved**.
- New pinned Ghidra instructions for FF7089FC, FF708770,
  FF705AF2 and FF705B16 will trace the boot-device source and
  special code 8. No ROM/SPL execution is implied.

## Fastboot power input and stock recovery path

- Fastboot getvar:power-source reads SARADC channel 0, scales the
  measurement with fixed-point factor 0x10624DD3 >>38 and classifies
  computed values into invalid / battery / ac / invalid-ac by
  thresholds 624999, 1089999, 1249999.
- SARADC channel 1 detects stock recovery-button values <=99.
  Original writer and board-setup reader pointers both target
  0x001462FC, and stock dorecovery environment selects fastboot 0.
- Physical button identity, voltage units and real ROM MaskROM
  entry have NOT been measured or verified.

## New independent firmware 3.10 image format research

The verified `libaz01-prog` ELF source contains
`az01_open_image` at 0x0001738C. It dispatches by the first
four file bytes to different format handlers:

- `AZ0x` (proprietary);
- `AZ01` (proprietary);
- `D0 0D FE ED`, the standard FDT header magic.

That third branch is consistent with a flattened-tree image such
as FIT, but header matching does NOT prove that a given FIT image
is accepted, signed, or checked under any particular key policy.

Upstream references for interpretation only:
- Linux libfdt defines FDT_MAGIC=0xd00dfeed:
  https://codebrowser.dev/linux/linux/scripts/dtc/libfdt/fdt.h.html
- U-Boot FIT design and separate signature controls:
  https://docs.u-boot.org/en/latest/usage/fit/signature.html

The next V11 probe captures original image PIC/GOT words around
the three format-handler candidate selection slots. Original
image words are *not* proven callable runtime function targets.

## Security: image verification has a state and option-dependent fast path

`az01_verify_image` at 0x00017554 maintains state
in its image object:
- Cached state 1 returns 0 without rechecking the format handler.
- Any nonzero state plus option bit 1 set returns 0 without
  rechecking. Known caller option values are 0 and 2.
- A callback return 0 stores state 1.
- A callback return -13 stores state 2 and returns -13.
- A subsequent option-2 invocation from state 2 returns success
  without repeating the cryptographic callback.

This can be modeled precisely on verified states/options, but
the *meaning* of error -13, whether the same image object is
reused, and format-specific policy remain UNKNOWN. Do not
infer a usable bypass or universal signature enforcement.

The downloaded-image path selects an onboard key lookup and
uses verify_image(...,0) before applying; the partition-write
orchestration calls verify_image(...,2) and may vary compatibility
checks depending on flags. Source code paths do not prove successful
vendor update execution or secure boot fuses.

## XZ compressed write pathways: verified size/memory distinction

`az01_write_partition_image` at 0x0001798C uses distinct
direct write, XZ-bootloader and XZ-partition paths:
- XZ bootloader path checks an indicated decoded size against
  0xA00000 (10 MiB) before allocating and calling
  lzma_stream_buffer_decode, then invokes write_bootloader.
  This is *not* a global bootloader maximum or a full overflow proof.
- XZ partition path initializes a streaming LZMA decoder,
  allocates **four 0x100000-byte (1 MiB) buffers**, launches a
  worker thread, and invokes az01_stream_partition.
  The count is four because the pointer advances from
  &local_e0 to local_dc[0], [1], [2] and [3] before stopping.
- The generic az01_stream_partition API returns -11 if its
  backend stream callback slot is null.
- The RockUSB-like backend table has that null stream slot,
  while Fastboot and FF/61/7A have corresponding callbacks.
  This establishes one API-compatibility distinction, not an
  overall transport inability to update firmware.

These decompression and stream paths were NOT exercised with
vendor inputs. Real size checks, header interpretation, callback
error propagation and commit lifecycle require more analysis.

## Next source-only research work

- New `synthesize_update_format_and_spl.py` checks 17
  independent static predicates and emits exact limited
  verification-state examples, updated by synthetic tests.
- New `test_update_format_and_spl.py` has eight synthetic
  tests for state transitions, format magic, XZ buffer counts,
  USB callback mismatch, source provenance and negatives.
- Source safety guards prevent accidental original vendor
  execution, model overclaims and Ghidra PIC handler mislabeling.
- Ghidra captures additional SPL device discovery helpers,
  libaz01 image-open/verification/XZ/signature ARM instructions,
  and original PIC format-handler candidate words.
- Broad seven-track matrix expands from 59 to **64 questions**.
  Focused direct-boot remains 42 questions across ten topics.
- Same integrated Ghidra/QEMU harness will produce
  `update-format-and-spl.json` and `.md` in a SHA-indexed private ZIP.

The latest changes were source-checked; the local offline
17-predicate analyzer was independently run against this owner
archive and five local positive/negative tests passed. The
repository's newly committed eight synthetic tests and full
integration still require a workstation execution.

## Recovery and installation boundaries

A functioning stock U-Boot can potentially provide a reversible
MPFree kernel development path, but Fastboot download permission,
safe bootm image format, RAM/FDT placement and actual physical
device entry are not established. The stock ADC recovery path
depends on functioning SPL/U-Boot. Independent RK3288 immutable
MaskROM entry, accepted owner SRAM loader and whole-eMMC
restoration remain unproven, and remain prerequisites before any
early-boot firmware writes.
