# MPFree V20 — Actual storage-backend callback and U-Boot command-parser investigation

Branch: research/bootchain-v20-storage-command-20261112

## Why V20 is the next experiment

The user's owner-private V19 archive
bootchain-wide-20261112T225445Z-v19-35117.zip completed in 14.11 seconds,
with all 13 indexed files SHA-verified.

It successfully executed 24/24 owner-P3 ARM guest experiments:
- Reboot modes 1 and 9 select vendor U-Boot recovery; mode 3 does not.
- PMU reboot-mode state is cleared by the tested function.
- The ADC threshold is strict: 99 selects recovery; 100 does not.
- Environment-backed getvar variant 4 invokes the environment accessor,
  variant 5 does not; both return without observed mocked persistence.
- The V18 getvar SIGSEGV incidents were an incomplete VM dependency fixture.

Ghidra further captured 837 P1 and 1850 P3 STORE dependency witnesses,
29/181 CALLIND witnesses, 366/2835 direct call edges, and 733 successfully
decompiled P3 functions within the selected 739-function region.

These results do NOT answer the chief MPFree objective: non-persistent external
kernel boot and actual identity of the storage backend that vendor OEM code
can invoke.

An actual source-derived and testable new hypothesis follows from V19's
decompilation of FUN_0010b74c:

- The function obtains a backend object b from FUN_0011c3ac.
- FUN_0011c3ec returns b+0xC8.
- The caller loads a function pointer at offset 0x68 from the projection.
- Thus the source-level storage callback resides at b+0x130.
- The concrete backend implementation and any eMMC/SD effects remain UNKNOWN.

V20 performs NEW guest tests of exactly this indirect dispatch and the
native U-Boot command-parser entry. It also performs a full owner-P3 Ghidra
High-Pcode search for potential registrations/writes of callback fields,
parser table entries and indirect calls; this is not a generic 85-keyword scan.

## One copy-paste terminal command (ML-tower)

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-v20-storage-command-20261112 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_v20.py" \
  "$R/tools/mpc-one/test_v20.py" \
  "$R/tools/ghidra/MPFreeV20BackendCensus.py" && \
arm-linux-gnueabihf-gcc \
  -std=c11 -O2 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/vendor_probe_v20.c" && \
python3 "$R/tools/mpc-one/test_v20.py" && \
python3 "$R/tools/mpc-one/run_v20.py"
~~~

The V20 runner auto-selects the newest private V19 ZIP in ~/mpfree-work.
Override with --prior-v19 /absolute/path/to/private-v19.zip.

It produces one private archive:

    ~/mpfree-work/bootchain-wide-<UTC>-v20-<PID>.zip

Upload only that ZIP. SHA-256 manifest is ZIP_INDEX.json and separate sidecar.
Do NOT commit the private Ghidra-derived reports or firmware to the repo.

## Investigations

### A. Full P3 native Ghidra field census (concurrent with QEMU)

On the owner's saved SHA-pinned P3 project, with -readOnly -noanalysis:
- Enumerate ALL recognized P3 functions and their true source instruction
  call graph; run High-Pcode decompilation and record all failures.
- Recover field loads/stores involving candidate offsets 0xC8, 0x68,
  0x130 and other storage-related struct fields.
- Record actual Ghidra expression dependencies, pointer math and
  instruction addresses; do not infer a concrete runtime address from an
  isolated literal.
- Inspect every CALLIND/BRANCHIND, with root neighborhood pseudocode.
- Build forward/reverse recursive call-graph reachability for backend
  lookup, persistence, command dispatch, image boot, MMC source and recovery.
- Emit V20-P3-BACKEND-COMMAND-FUNCTIONS.jsonl and verified summary.
- Export exact owner-P3 bytes to a temporary private directory after
  hash verification; let QEMU start WHILE Ghidra keeps scanning.
- Require >=96% recognized-function decompilation, at least one indirect
  and at least one callback-field store to label the stage successful.

### B. Genuine ARM execution: storage backend

Six scenarios execute U-Boot function FUN_0010b74c itself, with
synthetic backend object and mocked dependencies:

0. Backend unavailable.
1. Backend initialization fails.
2. Image staging fails.
3. Geometry/extent lookup fails.
4. Backend callback returns expected size.
5. Backend callback returns failure.

The owner ARM code derives the callback through the genuine
FUN_0011c3ec projection and invokes the synthesized b+0x130 function
pointer in guest memory. The callback is a counted safe stub; it does NOT
write any real media. Tests record branch outcomes, callback counts and
return values. A crash or unexpected value is a FAILURE, not an answer.

### C. Genuine ARM execution: U-Boot command parser

Sixteen cases execute the actual U-Boot command wrapper at 0x00110484,
which reaches parser 0x001095f8. Cases cover null, empty, echo with and
without newline, MMC selection syntax, filesystem load syntax, bootz
syntax and a saveenv word. Both wrapper mode-flag inputs are tested.

The final native command dispatcher FUN_00109b68 and alloc/free/persistence
side effects are replaced with in-guest stubs. The saveenv WORD is not
actually executed. This can establish parser reachability and argument
handling but does NOT prove booting from SD or physical persistence safety.

### D. 85-question evidence audit

All 85 V19 research question identifiers and previous classifications are
retained. Only exact questions that intersect the new P3 callback/command
research receive additional file-specific evidence pointers. Running an
analysis does not automatically mark a question solved.

## Important limitations

Generic ARM QEMU virt is NOT Rockchip RK3288 BootROM. Hardware GPIO, ADC,
eMMC, USB and DMA behavior cannot be verified from this VM alone.
Physical MPC Fastboot OEM unlock, write, saveenv, flash, erase or boot is
NEVER invoked here.

Only static firmware and original owner ARM instructions execute in private
guest memory with I/O callbacks stubbed. The original P1/P3/owner backups are
read-only; no new device-side changes occur. Source firmware bytes and
guest fixture images are temporary and intentionally excluded from the ZIP.

The scripts are committed to GitHub; their first owner-side QEMU/Ghidra run
is still necessary to confirm actual stage results. Host regressions and
ARM C cross-compilation run first. No artificial runtime delays are used.
