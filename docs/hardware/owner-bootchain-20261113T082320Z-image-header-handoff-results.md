# MPFree owner boot-chain audit — 2026-11-13T08:23:20Z

Private input: `bootchain-wide-integrated-20261113T082320Z-444016.zip`.
**Never commit** original vendor firmware, complete decompilation, extracted
rootfs or owner archive. This document records limited derived facts only.

## Verified owner archive

- ZIP CRC passed. 120 members, including index; **119/119** SHA-256 and
  size entries verified. No unindexed or duplicate members.
- **21/21** integrated research stages successful; broad Ghidra 53.8 s,
  V11 directed Ghidra 47.1 s; total 104.1 s.
- Generic ARM virt QEMU booted cloned 3.10 userspace. It did not execute
  original SPL, U-Boot, update executable, or immutable BootROM.
- USB synthetic guest tests: 13 interface cases and 21 endpoint cases.
- 67 broad questions across seven tracks, 42 focused across ten topics.
- No physical MPC, USB, ADC, eMMC, fuse/OTP or MaskROM command executed.

## Fastboot request/receive globals — identity independently proven

Original U-Boot SHA-256:
`b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a`.

- Fastboot download handler's expected-length literal at
  `0x001229F4` → `0x0014D684`.
- Receive handler's expected-length literal at
  `0x00122AD4` → **same** `0x0014D684`.
- Fastboot download handler's received-byte cursor literal at
  `0x001229F8` → `0x0014D680`.
- Receive handler's cursor literal at `0x00122AC4` →
  **same** `0x0014D680`.
- Receive routine copies to `0x00800800 + bytes_received`,
  capped by bytes still requested. Normal download is gated by
  the authorization helper and advertises up to `0x70000000`
  bytes (NOT known free/usable RAM).
- Boot completion dispatches into stock `bootm`, which has
  legacy CRC and FDT-structured image handling.
- This establishes **static dataflow pointer equality**. It does
  NOT establish initial runtime permission, safe RAM/FDT bounds,
  custom image acceptance or successful physical Fastboot boot.

## Major SPL finding — legacy image header controls second-stage branch

Original verified P1 SPL payload SHA-256:
`6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f`.

The preceding investigation found the second indirect handler
context at `0xFF70A244` and callback slot `0xFF70A250`,
whose **original-image word is zero**.

In the new owner archive, three original instruction references are
classified as WRITEs to that specific slot:

- `0xFF7052A8`: default context initialization;
- `0xFF7052DC`, `0xFF7052F2`: legacy image-header-derived writes.

The original SPL code supplies a concrete explanation:

1. `0xFF7054A4` loads a header and calls image parser `0xFF7052C0`.
2. Parser checks legacy U-Boot header magic stored at `0xFF705308`;
   the independent original P1 literal scan matches the magic.
3. Parser reads the image header's big-endian size (`+0x0C`),
   load address (`+0x10`) and entry point (`+0x14`).
4. Depending on a context flag, the parser places either the
   byte-swapped entry field or byte-swapped load field into
   context word `[3]` (`+0x0C`).
5. A default initializer sets context entry `+0x0C` to
   `0x00100000` in its fallback branch.
6. Boot-selection code invokes `FF705310(context)`, whose
   indirect branch dereferences `context + 0x0C`.

This provides **source-derived structural evidence** that the
second-stage target is filled from a loaded image header rather
than a permanently stored address. Initial zero is expected for
a runtime-populated handoff slot, not proof SPL executes address 0.

### Independent factory P3 image header constraint

The original P3 legacy header is independently validated:
- CRC32 header: valid; CRC32 payload: valid;
- load address: **`0x00100000`**;
- image entry field: **`0x00000000`**;
- image legacy type ID: **5**, ARM architecture ID **2**.

Therefore the unmodified factory P3 cannot safely execute by
blindly jumping to the entry-field value `0`. The SPL flag-dependent
load-address branch or another explicitly established adjustment
must apply. A valid CRC is NOT a cryptographic signature.

**Remaining static proof:** Read original literal-pool words at
`0xFF705308` (expected magic), `0xFF70530C` (header-parser
context pointer), `0xFF7052B8` (default-handler context pointer),
and the context flag word `context+0x14`. Prove that both handlers
indeed share `0xFF70A244` and determine the chosen branch for
the factory P3. The next Ghidra probe now captures these words
and the relevant ARM instructions.

**Hardware proof NOT established:** Original RK3288 BootROM
execution of this SPL, verified physical memory layout, acceptance
of an independently constructed P3, eMMC dual boot or restoration.

## 3.10 update formats: exact static PIC slot arithmetic

The `az01_open_image` function sets its internal PIC table base
to `0x0000B97C + 0x000173C0 = 0x00022D3C`, and applies
original image offsets `0x298` (AZ01), `0x2AC` (AZ0x),
and `0x2A4` (FDT magic).

Candidate original descriptor pointer slots are therefore:
- AZ01: **`0x00022FD4`**
- AZ0x: **`0x00022FE8`**
- FDT magic: **`0x00022FE0`**

These are *static PIC-table slot candidates*. They are **not
runtime-relocated callable function addresses**. Each first points
to a handler descriptor whose first word is a callback. Next V11
Ghidra pass reads original words and potential descriptor heads
from each candidate location without executing the vendor ELF.
Format-specific verification semantics remain unproven.

## Other tracks still open

- SPL boot-order table has five `0xDEADBEEF` original entries;
  first is populated from boot-source resolver. Codes 1 and 8
  are not yet mapped reliably to physical MMC controllers.
- Three USB backends have different stream/commit callback slots.
  FF/61/7A identity remains UNKNOWN, not silicon RockUSB.
- Firmware `az01_verify_image` caches verification state,
  and option 2 can return success from nonzero state without
  re-executing the format verifier. The `-13` error, object
  reuse and security impact remain unproven.
- XZ compressed bootloader branch checks a 10 MiB size bound,
  and compressed partition branch uses four 1 MiB buffers.
- Stock recovery-button SARADC channel 1 <=99 sets flag at
  `0x001462FC` and saved boot command uses `dorecovery`
  to choose Fastboot. This requires working SPL/U-Boot.
- Independent immutable MaskROM entry, SRAM loader acceptance,
  full eMMC restoration and ROM signature policy remain unproven.

## Code and next run

- New `synthesize_spl_image_handoff.py` cross-validates original
  SPL parser, three static WRITE references, image header layout
  and the separate factory P3 CRC/SHA metadata.
- Eight synthetic positive/negative regression tests.
- Expanded V11 Ghidra byte-level literal pointer captures for
  both SPL parser/fallback contexts and updater PIC table slots.
- Next integrated run will include a separate
  `spl-image-handoff-evidence` stage, increasing the expected
  count to **22** and the broad matrix to **70 questions**.
- Existing-ZIP preflight requires facts already measured in the
  old archive while reporting missing *new* Ghidra literals as
  unknown, not as verified.

No physical programming, Fastboot flash or MaskROM trigger should
be attempted on these static findings alone.
