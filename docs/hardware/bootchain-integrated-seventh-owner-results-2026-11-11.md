# MPFree integrated owner research — seventh run

**Owner-private input:** `bootchain-wide-integrated-20261112T065421Z-3834549.zip`  
**Run timestamp:** 2026-11-12T065421Z (2026-11-11 Pacific)  
**Important:** The ZIP, Ghidra pseudocode, original firmware, and exact console logs remain private and are **not** committed to this public source repository.

## Integrity and completed stages

- ZIP CRC passed, **89 of 89** SHA-256 indexed members checked and matched; 90 entries including the index.
- Wide SPL/U-Boot audit: passed, 55.2 seconds; 213 recognized SPL functions and 1050 recognized U-Boot functions.
- Targeted V11 analysis: passed, 47.1 seconds; 12 updater, 143 library, 25 selected SPL, 55 selected U-Boot functions decompiled.
- QEMU generic ARM `virt` booted a clone of the 3.10 rootfs. Both vendor ELF hashes matched the static analysis inputs exactly.
- Independent synthetic ARM USB interface-selection tests: **13/13 passed**.
- Independent synthetic ARM USB endpoint-discovery tests: **21/21 passed**.
- All automated evidence synthesis stages passed, including 3-backend comparison and FF/61/7A message framing.
- Total execution: **105.2 seconds**. No report-stage failures; no physical MPC device accessed; no original updater executable run.

## FF/61/7A wire-format evidence from direct review of Ghidra pseudocode

The USB interface class/subclass/protocol `ff/61/7a` selects constructor `0x1b49c`
and callback table `0x12b48`. The protocol remains **unidentified**.

Decompiled routine `0x1a314` directly supplies these structural observations:

| Relative offset | Interpretation and caveat |
| --- | --- |
| Frame `+0x04` | 32-bit message identifier compared with outstanding request |
| Frame `+0x08`, `+0x0c` | Additional 32-bit return/status fields; precise semantics unknown |
| Frame `+0x10` | 32-bit correlation value compared with outstanding request |
| Frame `+0x14` | 16-bit response flags |
| Frame `+0x16` | 16-bit declared frame length |
| Frame `+0x18` | Optional error/status word examined by some branches |
| Context `+0x20` | 16-bit incrementing sequence counter used during request creation |

The receive loop checks **at least 24 bytes** and a declared-length bound,
matches message ID and correlation value to the outstanding request,
and only exits the loop after flag **bit 0** is set.
Flag **bit 1** routes to error-handling logic; additional flag-bit
conditions and status values influence the success/error result.
Some branches return `-11` or `-9`. Assigning the semantic labels
`ACK`, `retry`, `fatal`, etc. would be speculative.

**Critical correction to the previous generated framing report:** The six
`0x5aa5fe01`, `0x5aa5fd02`, `0x5aa5fc03`, `0x5aa5fb04`,
`0x5aa5fa05`, and `0x5aa5f906` words are **equality-compared message IDs**
inside `0x1a674` and `0x1a314`.
The `0xa55a0101` and `0xa55a0202` words are **constructed outgoing values**
in `0x1a8d0`, not comparison-tested identifiers.
`0x5aa5fc04` is a range-comparison bound, not an observed opcode.

Ghidra currently prints `void` return signatures for at least
`0x1a674`, `0x1d644`, and `0x1aabc`, although callers consume
their apparent status results. These are likely Ghidra type-inference
limitations. The next Ghidra pass will export bounded ARM machine
instruction sequences to verify the actual calling conventions and
callsite return-value checks; nothing about these type mismatches
establishes a signature bypass or unsafe physical boot mode.

The `0x1d644` Linux usbdevfs layer calls `USBDEVFS_SUBMITURB` and
`USBDEVFS_REAPURBNDELAY`. This is *host USB I/O infrastructure*,
not a device protocol or proof of ROM execution.

## New additions to the next integrated run

1. The FF/61/7A framing synthesizer now differentiates six
   compared message IDs, two constructed control words, and one
   non-opcode range bound.
2. It records eight explicit response-validation checks and reports
   observed decompiler return-type uncertainty.
3. The V11 Ghidra script captures bounded raw ARM instructions for
   relevant response-parser, request-builder, USB transfer and
   completion functions in the private report. The Ghidra source
   remains SHA-pinned and the owner ZIP remains private.
4. Synthetic regression fixtures validate the classification,
   response-check extraction and decompiler-warning reporting.

## Constraints and next milestone

The corrected metadata should support design of a **no-device**,
read-only FF/61/7A fake response endpoint. Actual original vendor
code should only be evaluated later in a separately hardened outer
KVM VM with no host devices, networking or mutable firmware storage,
using an explicit denying fake transport.

Independent hardware recovery is still **not established**: there is
no verified immutable RK3288 MaskROM entry, owner-built SPL execution,
or reversible eMMC restoration on the physical MPC One.
