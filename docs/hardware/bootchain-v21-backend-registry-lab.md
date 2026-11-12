# MPFree V21 — Identify actual U-Boot backend registration and debug indirect storage callbacks

Branch: research/bootchain-v21-backend-registry-20261112

## Evidence that determines the new experiments

The SHA-indexed owner V20 ZIP
bootchain-wide-20261112T232001Z-v20-102871.zip was inspected and all
nine indexed files passed SHA-256 and ZIP CRC verification.

V20 completed in 17.89 seconds. Its full U-Boot Ghidra P3 census
decompiled 1044 of 1050 recognized functions, identified 2835 direct
call edges, 278 indirect call sites, 116 selected STORE sites and 221
selected LOAD sites. Six functions near the P3 entry retain unrelated
Ghidra address-space errors.

V20 ARM virt guest: 20/22 tests passed. All sixteen command-parser cases
passed with the final COMMAND DISPATCHER REPLACED. Four early-exit
storage-backend cases passed. But the two crucial success-path cases,
v20_storage_4 and v20_storage_5, failed with **Illegal instruction**.
They did not produce the ordinary guest result JSON. Neither actual
success-path indirect callback has therefore been validated. The Ghidra
stage itself succeeded and this does not demonstrate a physical MPC crash.

New static evidence from V20:
- Original FUN_0011c3ac searches an intrusive backend linked list.
- Each node has a next pointer at offset 0, device ID at offset 0xCC.
- The list head/sentinel pointer is read via the owner P3 literal cell
  at 0x0011C3D4.
- FUN_0011c3ec projects a backend pointer by +0xC8.
- FUN_0010b74c dereferences function pointer [backend+0x130] and calls
  it indirectly at instruction 0x0010B7D6.
- Real backend provider, concrete callback implementation and physical
  destination of that callback remain UNKNOWN.

V21 is NOT another pass of the same V20 four early-exit cases. It
targets the original linked-list lookup, genuine projection, original
success callback, and **fault program counter** evidence.

## One copy-paste ML-tower command

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-v21-backend-registry-20261112 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_v21.py" \
  "$R/tools/mpc-one/test_v21.py" \
  "$R/tools/ghidra/MPFreeV21BackendRegistry.py" && \
arm-linux-gnueabihf-gcc \
  -std=c11 -O2 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/vendor_probe_v21.c" && \
python3 "$R/tools/mpc-one/test_v21.py" && \
python3 "$R/tools/mpc-one/run_v21.py"
~~~

The runner auto-selects the newest private V20 ZIP in ~/mpfree-work.
To use another, pass --prior-v20 /absolute/path/to/v20.zip.

Output is exactly ONE private report ZIP:

    ~/mpfree-work/bootchain-wide-<UTC>-v21-<PID>.zip

The ZIP is SHA-256 indexed and verified after writing; source firmware
and all private temporary QEMU fixtures are excluded.

## New concurrent investigations

### 1. Full P3 Ghidra backend-registry value-flow audit

SHA-pinned original owner 3.10.0 U-Boot saved project, processed
-readOnly -noanalysis. Ghidra explicitly validates the owner image
before exporting a private temporary copy to the guest. QEMU starts
when that verified export is ready, DURING the long P3 analysis.

Unlike the V20 script, which primarily tracked storage **addresses**
and relevant constant offsets, V21 examines both sides of every
High P-code STORE:
- Destination expression including source instructions and constants.
- **Actual stored value** dependency: constants, symbol references,
  ADD/SUB, CALL output, LOAD, PHI and unbound runtime input.
- Function pointer constants, registration field offsets 0xCC,
  0xC8, 0x130, 0x68, 0x6C and surrounding backend metadata.
- Complete recursively reachable caller/callee graph from device
  registration, image loading, MMC boot, U-Boot parser and OEM handler.
- The list-head sentinel's literal cell, data and code xrefs.
- Static potential function pointer data tables.
- Whitelisted bootcmd/boot_targets/mmc/ext4load/bootz string
  locations and their Ghidra xrefs, without publishing auth secrets.
- Full owner-private pseudocode for backend registration candidates
  (especially FUN_0011d048, FUN_00116f64 and FUN_0011dcb0),
  actual backend lookup, boot selection and command-dispatch functions.

The output contains all relevant STORE-value evidence and a second
deep source report; it is not a one-page keyword summary.
Actual runtime backend identity is NOT assumed from a static pointer
candidate.

### 2. Actual owner ARM backend registry in QEMU

Five NEW genuine U-Boot function cases execute FUN_0011c3ac
with a synthetic intrusive list (two fake backend IDs), including
first node, second node, missing ID, empty list, and
FUN_0011c3ec projection. Each case checks the actual returned
address and records the fake list-head pointer. A missing lookup
must be accounted for, not mistaken for a successful lookup.

### 3. Actual owner ARM storage callback diagnostics

Six more NEW cases investigate the two failed V20 success paths.
The test harness now:
- Verifies callback stub reachability by invoking it directly first.
- Records the exact backend+0x130 pointer and the projected
  (backend+0xC8)+0x68 address BEFORE calling owner storage code.
- Executes the genuine safe FUN_0010b728, which initializes the
  output extent, instead of V20's stub that left that stack value
  undefined.
- Exercises callback returns 0, 16 and 32 and records owner results.
- Captures SIGILL/SIGSEGV/SIGBUS with ARM PC, LR, R5, R8, R0, CPSR,
  plus current experimental phase, even if a case cannot return.
- Keeps media writes and callbacks as guest counter/return stubs.
- Requires observed per-case test JSON for success. A guest signal
  is a diagnosed FAIL, not a passing callback or physical defect.

These are 11 new guest scenarios, not 11 completed research questions.
All previously completed V17/V18/V19/V20 ARM cases are reused as
SHA-verified owner evidence, not counted again.

### 4. Explicit evidence audit

V21 retains all original 85 research question IDs. Only relevant
questions get exact report-file references. Function-pointer
registration candidates are ranked separately from proven registered
methods. Zero unverified questions are auto-closed on name matching.

## Interpretation / safety

This VM is generic ARM virt, NOT physical RK3288 MaskROM. No real
Fastboot USB, SD, eMMC, framebuffer, firmware write, U-Boot saveenv,
flash, erase, OTP/eFuse operation or kernel handoff is executed.
Only selected verified owner instructions execute with synthetic RAM
and I/O callbacks replaced. A guest SIGILL is not a physical-MPC
fault without additional independent evidence.

Source is committed, but the V21 owner's Ghidra/QEMU run has NOT
yet been executed on the ML-tower. The user command compiles/tests
before expensive work, and the resulting ZIP records actual success
and failure. No artificial sleep is used to inflate runtime.
