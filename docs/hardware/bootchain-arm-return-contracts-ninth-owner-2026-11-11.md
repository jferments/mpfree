# MPFree ninth owner follow-up — audited ARM return-contract evidence

Date: 2026-11-12 UTC (2026-11-11 Pacific).
Owner-private input: `mpfree-arm-return-contracts-20261112T080830Z.zip`.
This derived ZIP was generated from
`bootchain-wide-integrated-20261112T072337Z-3874253.zip`.

The private ZIP, firmware ELF objects, decompiled function bodies and raw
Ghidra instruction reports remain **outside the public repository**.

## Verification

- Uploaded derived ZIP CRC passed.
- Its two indexed reports (JSON + Markdown) matched the index's SHA-256
  hashes and sizes. Third member is the index itself.
- The derived report records verification of **89 indexed files** in its
  original integrated Ghidra ZIP. The library ELF source SHA-256 remains
  `af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623`.
- Eight previously identified response-parser predicates are present in
  the derived framing evidence. This is **static** source analysis,
  not a runtime exercise of AKAI's updater.

## Correction: return-result usage is more extensive than first reported

An audit of **11 captured direct BL callsites** finds the following
distinct behaviors:

| Return usage category | Count | Confidence |
| --- | ---: | --- |
| Explicit `cmp r0,...` followed by conditional branch | **2** | Direct ARM evidence |
| `subs rN,r0,#0` followed by conditional branch | **4** | Direct ARM evidence |
| `cpy r4,r0` without an immediate conditional branch | **2** | Return preserved; downstream interpretation unverified |
| Callee's r0 carried through caller's normal fallthrough epilogue | **3** | Direct ARM instruction sequence and return register preservation |

The previous analyzer mistakenly counted only the two explicit
`CMP` cases as return-value checks. The `SUBS` mnemonic *also*
sets the ARM condition flags; subtracting zero from `r0` is an
equivalent zero/nonzero test and copies the value into a destination
register at the same time.

### Verified USB transport helper (ELF `0x1d644`)

All **five captured callsites** use the transport helper's integer
return: four `SUBS`-zero checks, and one explicit `CMP`.
Among them:

- Caller `0x1a178` at `0x1a270`: `subs r3,r0,#0` then `beq`.
- Caller `0x1a314` at `0x1a3bc`: `subs r5,r0,#0` then `bne`.
- Caller `0x1a674` at `0x1a7a0`: `cmp r0,#0` then `bne`.
- Caller `0x1afd4` at `0x1b054`: `subs r4,r0,#0` then `beq`.
- Caller `0x1afd4` at `0x1b108`: `subs r4,r0,#0` then `bne`.

This is stronger independent corroboration that Ghidra's current
`void` inference for `0x1d644` is not a reliable description
of its caller-visible return contract.

### Framed command helper `0x1a674`

Three captured callsites:
one at `0x1b3fc` immediately tests `r0` with `cmp`/ `blt`;
the other two do not immediately test it in the short captured window.
Their eventual dataflow is unresolved.

### Framed response parser `0x1a314`

Three captured callsites: two in `0x1afd4` copy `r0` into
`r4` shortly after the call, including one following an unrelated
floating-point move. That proves result capture; **it does not**
by itself prove rejection, success, retry, or storage permissions.
The third call is from `0x1a674` at `0x1a7cc`; on its normal
fallthrough path, its `r0` survives the stack-canary check and
function return at `0x1a80c`.

### Three normal-fallthrough status propagation paths

The complete instruction listings (not only the first three
post-call instructions) resolve the prior three unknowns:

- `0x1a8d0`: `bl 0x1a674` at `0x1a964`; following
  stack-canary check and `ldr pc,[sp],#4` at `0x1a990`
  preserve `r0` on the normal return path.
- `0x1aabc`: `bl 0x1a674` at `0x1aba8`; normal return
  at `0x1abe8` preserves `r0`.
- `0x1a674`: `bl 0x1a314` at `0x1a7cc`; normal return
  at `0x1a80c` preserves `r0`.

The intervening `bne` operations are stack-canary failure exits;
their exceptional paths have not been treated as ordinary integer
returns. This finding verifies **normal fallthrough only**, not
every possible path or the exact meaning of every status code.

## Tooling corrections committed to research branch

- `tools/mpc-one/analyze_arm_return_contracts.py` now distinguishes
  explicit status compares, `SUBS`-zero status tests, and copies of `r0`.
- It reports original explicit-CMP counts separately from all immediate
  status-test callsites, avoiding misleading backward-compatibility fields.
- It conservatively describes `CPY/MOV rN,r0` as a preserved return
  requiring later dataflow analysis, and detects straight-line normal
  epilogues retaining the callee's `r0` without intervening writes or calls.
- `tools/mpc-one/tests/test_arm_return_contracts.py` includes synthetic
  reproductions of all four `SUBS` checks, two capture shapes,
  three normal-return pass-throughs, clobber/call counterexamples and
  an explicit-CMP case.
- `tools/mpc-one/run_bootchain_integrated_lab.py` also uses these
  revised categories in future private output ZIPs.

No direct vendor code execution or physical MPC/USB/eMMC writes
were attempted by any of these tools.

## Recommended next research

The 11 captured direct callsites are now provisionally categorized:
six immediate flag tests, two deferred register copies and three
normal-return pass-throughs. For future static analysis, follow any
unresolved *downstream uses of the copied r4 values* and distinguish
specific negative result codes by their actual control-flow branches.
Continue developing the independent in-memory fake response model
before any original updater execution.

Independent MPC recovery is still blocked on real-hardware proof of
genuine RK3288 MaskROM entry, owner-controlled SRAM loader execution
and reversible eMMC restore. None of the return-contract findings
establishes those capabilities.
