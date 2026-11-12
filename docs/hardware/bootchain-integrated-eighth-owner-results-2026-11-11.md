# MPFree eighth private integrated bootchain run — ARM machine-code return contracts

Date (Pacific): 2026-11-11.
Owner-private archive: `bootchain-wide-integrated-20261112T072337Z-3874253.zip`.
**The full ZIP, proprietary Ghidra pseudocode, and original firmware
binaries are NOT committed to GitHub.** This document contains only
bounded technical conclusions supported by independently verified owner
evidence.

## Run integrity and isolated execution

- ZIP CRC check passed; **89/89 SHA-256 indexed members** verified.
- Full integrated session completed in **107.0 seconds**, with no reported
  stage failure. Wide Ghidra 55.0 s; targeted V11 Ghidra 48.9 s.
- 213 SPL and 1050 U-Boot recognized functions in broad audit.
- 143 updater-library functions in V11 directed decompilation.
- Generic ARM QEMU guest booted cloned 3.10 Linux filesystem.
- USB interface-selection model **13/13**, endpoint model **21/21**.
- Both analyzed vendor ELF SHA-256 hashes matched guest copies exactly.
- **832 ARM instructions** collected for targeted `FF/61/7A`
  framing, response and USB I/O routines.
- Neither original updater code nor SPL/U-Boot/RK3288 BootROM ran inside
  the QEMU guest. No physical MPC, USB passthrough or firmware writes.

## New machine-code corroboration: Ghidra void signatures are inconsistent

Original ELF library source SHA-256:
`af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623`.

### Linux usbdevfs transport: `0x0001d644`

Ghidra rendered this function as returning `void`, even though its
captured ARM instructions explicitly write status values into `r0`:

| ARM address | Instruction | Value placed in r0 |
| --- | --- | --- |
| `0x1d744` | `mov r0,#0x0` | 0 |
| `0x1d79c` | `mvn r0,#0x8` | -9 |
| `0x1d7b4` | `mvn r0,#0x8` | -9 |
| `0x1d7bc` | `mvn r0,#0x5` | -6 |

The function has a shared return epilogue at `0x1d770–0x1d788`.
These are concrete register writes and associated return-path candidates;
precise reachability and error meaning are separate questions.
The values arise in a routine that uses Linux USB ioctl submission/reaping
and `poll()`; they are **not USB protocol opcodes**.

### Framed command helper: `0x0001a674`

The instruction sequence at `0x1a7a0` calls `0x1d644`; the next
instructions at `0x1a7a4/0x1a7a8` compare `r0` with zero and
conditionally branch on a nonzero result.

The backend callback at `0x1b35c` calls `0x1a674` at `0x1b3fc`,
then executes `cmp r0,#0x0` at `0x1b400` and `blt` at `0x1b404`.
This is **direct caller machine-code evidence** that the negative result
of `0x1a674` is interpreted as failure.

The documented control-flow chain is therefore:
`0x1d644` (USB transport) → `0x1a674` (framed transfer) →
`0x1b35c` (backend callback). Return types inferred by the
decompiler should not override real callsite register usage.

Ghidra also marked `0x1aabc` as void, but that routine's caller paths
and preserved `r0` make its integer status semantics likely. Direct
machine-code return-contract proof for all paths is still incomplete.

### Response parser: `0x0001a314`

The captured ARM instruction region at `0x1a4a4–0x1a4d4` corroborates:
matching 32-bit words at frame offsets `+0x04` and `+0x10`, testing
the low two flag bits at `+0x14`, and looping on mismatch/incomplete
responses. On other paths, the parser checks a response status word at
`+0x18`. The minimum-length bound and the declared length at `+0x16`
remain supported by the V11 decompilation.

Do not assign labels such as ACK/NAK/retry/fatal beyond observed behavior.
The actual meaning of every message type and the special bit-2 validation
branch still requires further investigation.

## Tooling: no need to rerun full Ghidra for this evidence

Added source-only offline forensic tool:
`tools/mpc-one/analyze_arm_return_contracts.py`.

It takes a **previously generated** private integrated ZIP, verifies
the entire SHA-256 index and CRC, and writes a separate owner-private
ZIP containing `arm-return-contracts.json`,
`arm-return-contracts.md`, and an index of their hashes. It does not
execute Ghidra, QEMU, original firmware, USB commands or host
storage operations. Tests under
`tools/mpc-one/tests/test_arm_return_contracts.py`
use synthetic functions and tiny ZIP fixtures.

Future full integrated runs also incorporate this ARM return-contract
analysis automatically, so the owner need not run another standalone
process for later reports.

## In-memory FF/61/7A synthetic response model (source-only)

The branch now contains
`tools/mpc-one/ff617a_synthetic_memory_model.py` and
`tools/mpc-one/tests/test_ff617a_synthetic_memory_model.py`.
Its 14 independent synthetic cases passed locally; the exact committed
version still needs execution on the owner's workstation.

The model does not open any device, invoke ioctl or create a USB server;
it only parses caller-supplied Python `bytes` in memory. It checks:
an observed six-value message-ID set, 24-byte header minimum, 16-bit
declared length bound, 32-bit request-correlation field, a completion
bit, explicit error-word presence, and unknown flags. The constructed
`0xa55a...` values and `0x5aa5fc04` bound are rejected as incoming
message identifiers.

A synthetically matching frame is labeled **MATCHED_UNVERIFIED**,
never interpreted as proof of AKAI acceptance or a permitted firmware
command. The semantics of bit 2 are unresolved, so frames with it set
are classified **UNKNOWN_FLAGS**. A bounded replay loop makes this
useful for exercising rejection and status-handling rules without live
USB/device access or a firmware interpreter.

This is scaffolding for a future isolated fake transport, not an
implementation of a verified physical Rockchip protocol.

## Project-level limits and next real milestones

The static protocol understanding is now sufficient to plan an
**in-memory, deny-by-default virtual response matcher** for `FF/61/7A`.
It should accept only explicitly synthetic read-only fixtures, reject
unknown/mutating message IDs, and never open live USB or storage devices.
Original vendor updater execution is still deferred pending a hardened
outer virtualization boundary with no physical passthrough.

Independent MPC boot and restoration remain **unproven**: physical
silicon MaskROM access, execution of an owner-controlled first-stage
loader and independently verified eMMC recovery all require separate
real-hardware evidence. No boot-partition or OTP modification is
authorized by these static analyses.
