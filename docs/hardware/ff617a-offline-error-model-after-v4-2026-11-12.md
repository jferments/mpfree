# MPFree FF/61/7A — v4 return-contract closure and offline error-state model

**Owner-private evidence:** `mpfree-arm-return-contracts-v4-20261112T085952Z.zip` and
`bootchain-wide-integrated-20261112T072337Z-3874253.zip`.
Do **not** commit these archives, original ELF files, complete ARM
disassemblies or proprietary Ghidra pseudocode to this repository.

## v4 report integrity

The v4 derived ZIP CRC passed; its two report members exactly matched
their SHA-256 index entries. The report records prior successful
verification of all **89** indexed members of the original integrated
Ghidra/QEMU ZIP. Its complete available evidence covers **11 captured
direct callsites**:

| Evidence classification | Direct calls |
| --- | ---: |
| Explicit CMP return-value tests | 2 |
| SUBS-zero return-value tests | 4 |
| Direct r0 preserved through normal function return | 3 |
| Return copied r0→r4→r0 at shared normal exit | 2 |
| **Total classified** | **11** |

The two r4 traces are inside completion routine `0x1afd4`:
response-parser calls at `0x1b0c4` and `0x1b134` store results
at `0x1b0cc` and `0x1b138`. The common exit restores r4 into
r0 at `0x1b084`, then returns at `0x1b09c`.
The first path assumes an indirect callback preserves r4 per AAPCS.
Stack-canary failure and other exceptional paths were **not** proved
to return identically.

## Important correction: diagnostic IDs are NOT an input allowlist

Static instruction review of the original library's
`0x1a314` response parser separates **trace labeling** from
**request/response matching**.

Six constants are compared as identifiers to choose descriptive logging
labels: `0x5aa5fe01`, `0x5aa5fd02`, `0x5aa5fc03`,
`0x5aa5fb04`, `0x5aa5fa05`, `0x5aa5f906`.
These comparisons occur before logging. The parser's **actual
response-matching condition** compares the received 32-bit word at
`frame+4` to the outstanding request's corresponding word and
compares the 32-bit correlation value at `frame+0x10` against the
requested value. An otherwise matching identifier not in the six-value
diagnostic set is not explicitly rejected at this gate.

Consequently, the six-ID whitelist used by the older
`ff617a_synthetic_memory_model.py` is an intentional **conservative
MPFree policy**, NOT a verified vendor acceptance rule. We retain this
policy model for deny-by-default experiments, but we do not cite its
behavior as evidence of what the original firmware accepts.

## New offline branch model (committed; not vendor execution)

`tools/mpc-one/ff617a_offline_error_model.py` is a separate,
independently written synthetic response/error state interpreter.
It has **no USB transport, file I/O, networking or device access**.
The 17 synthetic regression tests in
`tools/mpc-one/tests/test_ff617a_offline_error_model.py`
passed locally before source-only commit; run them again on the
owner's workstation to verify the exact committed version.

The model covers these machine-code-informed paths:

| Original evidence | Synthetic behavior modeled |
| --- | --- |
| `0x1d644` and `0x1a314` | USB transport status nonzero returned by the parser |
| `0x1a3c8–0x1a3dc` | Received frame shorter than 24-byte minimum or declared length → receive again |
| `0x1a4a4–0x1a4c0` | Mismatched message ID or correlation value → receive again |
| `0x1a4c4–0x1a4cc` | Completion bit 0 absent → receive again |
| `0x1a4d0–0x1a4f4` | Error flag bit 1 with 32-bit error word 1 → -11; other/missing status → -9 |
| `0x1a5c8–0x1a5e0` | Request-mode bit 0 must agree with response flag bit 2, otherwise -9 |
| `0x1a5e4–0x1a618` | Nonzero response words at +8/+0xc require an output buffer; otherwise -9 |
| `0x1a518–0x1a538` | Observed integer result returned through r0 |

The model records a returned zero as **synthetic matching-path
behavior**, not proof that the actual updater would complete an
operation. Unknown high flag bits are marked **inconclusive**,
and bounded synthetic replay prevents infinite receive-loop tests.
A matching previously unknown diagnostic identifier is not rejected
on identity alone: this mirrors the observed request/response equality
gate but does not authorize physical device commands.

## Project-level boundaries

- Synthetic error-model results are not executable proofs of vendor behavior.
- Vendor `-6` is associated with a negative input length in the USB helper.
  `-9` occurs on transport timeout/errors and several protocol
  rejection paths. `-11` appears when the response error-status word
  equals 1. These numerical observations do not prove English meanings
  of the underlying device condition.
- A physical MPC, stock bootloader, MaskROM USB state, eMMC writes and
  OTP are entirely outside the scope of this research tool.
- Future controlled dynamic experiments would need a separately hardened
  outer VM with no physical USB/eMMC passthrough and an explicit,
  deny-by-default fake transport. The current code executes no vendor code.

The main independent-boot/recovery gates remain physical MaskROM entry,
owner-built SRAM loader acceptance and independently verified eMMC
restoration; none are established by this offline work.
