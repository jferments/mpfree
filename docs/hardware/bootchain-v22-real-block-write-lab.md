# MPFree V22 — Actual vendor U-Boot block-write driver and corrected ARM division

Branch: research/bootchain-v22-real-storage-write-20261112

## Exact evidence for this round

The uploaded V21 owner-private research ZIP
bootchain-wide-20261112T234542Z-v21-141079.zip was verified: valid CRC,
11 indexed files, all SHA-256 checks match. Execution took 15.43 seconds.

Static P3 research completed 1044/1050 recognized U-Boot decompilations,
3213 STORE operations, 467 stored-value witnesses and 278 indirect sites.
Only 6/11 ARM guest cases passed. The 5/5 genuine linked-list tests did
establish synthetic backend lookup behavior and +0xC8 interface projection.

All five failing real storage-wrapper cases trapped with SIGILL:
PC=0x0012EE08, LR=0x0010B7BB, CPSR=0x000F0010 (ARM state, Thumb bit 0).
The VM had intercepted address 0x0012EE08 with a **Thumb-only** stub.
But actual owner U-Boot code at 0x0012EE08 is an ARM-mode unsigned
division routine, called twice by FUN_0010B74C to compute sector counts.
It is not a device-driver callback. V22 does NOT intercept it.

More importantly, detailed reanalysis of the original decompiled firmware
recovered a **concrete registered storage write callback** missed by the
V21 summary:

  FUN_0011C9A8 -> FUN_0011D8E8 -> FUN_0011C434
                -> backend allocated at size 0x148
                -> interface projection b+0xC8
                -> interface[0x68] assigned from DAT_0011D95C
                -> owner static pointer 0x0011DE51 (Thumb)
                -> FUN_0011DE50, REAL U-Boot block-write implementation
                -> FUN_0011CA78, indirect lower-level driver request.

The adjacent erase function pointer at DAT_0011D960 points to
0x0011DD4D (FUN_0011DD4C). These owner-literal function targets are
already present in V21's SHA-pinned function-pointer inventory.

FUN_0011DE50 is a concrete real callback, but it does NOT prove a
specific physical eMMC or SD host instance without tracing the live
device-driver registration and selecting an actual MPC device.

## One ML-tower command

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-v22-real-storage-write-20261112 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_v22.py" \
  "$R/tools/mpc-one/test_v22.py" \
  "$R/tools/ghidra/MPFreeV22BlockTransport.py" && \
arm-linux-gnueabihf-gcc \
  -std=c11 -O2 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/vendor_probe_v22.c" && \
python3 "$R/tools/mpc-one/test_v22.py" && \
python3 "$R/tools/mpc-one/run_v22.py"
~~~

The runner auto-selects the latest private V21 ZIP from ~/mpfree-work.
Alternatively: python3 tools/mpc-one/run_v22.py --prior-v21 /absolute/path.zip.

Upload only the one generated owner-private ZIP:

    ~/mpfree-work/bootchain-wide-<UTC>-v22-<PID>.zip

## Ghidra and ARM guest truly concurrent

The owner P3 script reads the previously analyzed saved Ghidra project in
-readOnly -noanalysis mode. It verifies the exact raw 363649-byte P3 image
SHA-256 and exports one private copy into a temporary working directory;
that signals the VM guest to start WHILE source decompilation continues.

New targeted Ghidra investigations:
- Recursive caller and callee closure around actual 0x148-byte block
  object allocation, registration and linked-list search.
- Real write/erase callbacks, checked against raw owner pointer literals.
- Read/erase/write interface projection and real underlying driver method.
- FUN_0011CA78's indirect device-driver request path, including its
  secondary descriptor and transport callback pointers.
- Actual write handler control flow, transport invocation and bounds.
- Original ARM division code 0x0012EE08 and the wrapper's callsites.
- U-Boot MMC, bootz/bootm and command-source functions, including
  previously unresolved removable-media selection.
- Typed High P-code STORE and CALLIND source/value dependencies, direct
  graph edges, original source instructions and unexplained live inputs.

The script does NOT merely repeat V21's backend literal count. The new
decompilation roots specifically target registered physical write/erase
implementations and driver indirection not traced before.

## 29 distinct NEW genuine-owner ARM guest scenarios

A. **12 direct original ARM division tests.** Execute the original,
unmodified ARM-state 0x0012EE08 routine with exact quotient checks,
including 8192/512, 98304/512, overflow-adjacent and uneven operands.
This proves the emulator is no longer patching the true division routine
with Thumb instructions.

B. **8 original U-Boot persistence-wrapper tests.** Execute real
FUN_0010B74C through real division and the owner b+0xC8+0x68 callback,
using an instrumented, completely non-writing guest C function. Validate
callback ABI: projected interface pointer, computed start/count sectors,
buffer pointer, callback count and owner return status across block sizes.

C. **9 original registered block-write implementation tests.** Actually
execute owner FUN_0011DE50 (the newly resolved literal target) with a
synthetic backend registered through the genuine linked-list memory
layout. Test one block, multichunk write, out-of-range start/count,
missing backend, readiness failure, lower-level transport failure, zero
blocks and one-block chunks.

The low-level U-Boot driver command gateway FUN_0011CA78 is intercepted
in guest memory in EVERY block-write test. It does not reach real
hardware, USB, physical MMC, DMA, persistent environment storage or
a raw disk. Tests record the number of hypothetical transport commands
and owner-reported blocks without making a real media write.

All 29 new cases are distinct from earlier V17/V18/V19/V20/V21 cases.
There is no assertion that 29 different research questions are solved.

## One private report ZIP

- V22-STAGE-MANIFEST.json: real measured timings, SHA lineage, stage status.
- V22-REAL-OWNER-ARM-RUNTIME.json: every real ARM case, crashes retained.
- v22-arm-vendor-console.log: full QEMU boot and guest runtime evidence.
- deep-p3/V22-p3-SUMMARY.json: owner verified callback/erase pointer and graph.
- deep-p3/V22-p3-PATH-WITNESSES.jsonl: detailed per-function High P-code.
- V22-CONCRETE-FINDINGS-AND-85-QUESTIONS.json: exact new evidence and all
  prior question identifiers, no auto-closed hardware questions.
- ZIP_INDEX.json: SHA-256 and size for every included private file.

## Safety / physical proof limitations

No physical MPC, direct Fastboot transport, real U-Boot command dispatch,
eMMC, SD, MMC, USB, OTP, MaskROM, or firmware flash operation occurs.
Ghidra uses a read-only saved project. ARM guest uses generic virt (not
RK3288) with no network or host device passthrough, and only memory-backed
mock commands. Proprietary source firmware is confined to a private
temporary image and is deleted, not archived.

The new code and tests have been committed but **the full V22 suite has
not yet executed against the owner's local Ghidra/QEMU environment**.
Host syntax/compiler/tests are part of the copy/paste command. Failed
real code paths remain failures and must be diagnosed using the ZIP.
