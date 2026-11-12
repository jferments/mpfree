# MPFree V18 — Repair actual V17 Ghidra dataflow and investigate new ARM paths

Branch: research/bootchain-v18-dataflow-repair-20261112

## Verified V17 failure

The owner's SHA-indexed V17 ZIP passed integrity verification. It ran 1,239
isolated ARM vendor-code tests in 45.68 seconds. However, the Ghidra fixed-point
analyzer referenced possible_constant_targets() BEFORE defining that Python
function. Consequently, **667 genuine analysis attempts failed with a
NameError**: 103 P1 and 564 P3. An additional six P3 functions encountered
real Ghidra address-space errors. Both supposed indirect/MMIO evidence
files were empty. These were not genuine zero-observation findings.

V18 fixes the incorrect definition order and rejects a Ghidra stage unless
at least 96% of Ghidra-recognized functions decompile and its STORE/CALLIND/
BRANCHIND evidence file is nonempty. This recovers a large amount of
uncollected P-code data; it is not a repeat of the functional V17 subset.

## One ML-tower command

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-v18-dataflow-repair-20261112 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_v18.py" \
  "$R/tools/mpc-one/test_v18.py" \
  "$R/tools/ghidra/MPFreeV17FixedPointDive.py" && \
arm-linux-gnueabihf-gcc \
  -std=c11 -O2 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/vendor_probe_v18.c" && \
python3 "$R/tools/mpc-one/test_v18.py" && \
python3 "$R/tools/mpc-one/run_v18.py"
~~~

Requires the SHA-indexed private V17 ZIP under ~/mpfree-work. The script
can also take --prior-v17 /absolute/path/to/previous-v17.zip.

Output: ONE private
~/mpfree-work/bootchain-wide-<UTC>-v18-<PID>.zip.

## Workstreams

1. Concurrent P1 and P3 Ghidra exhaustive decompilation and recursive
   constant-propagation, including actual store and indirect-call candidate
   capture, failed-function accountability, and input SHA verification.

2. Genuine ARM QEMU vendor-code execution, started concurrently with the
   P3 Ghidra sweep once its SHA-verified private image is exported.
   Sixteen distinct new handler scenarios:
   twelve GETVAR built-in paths (six built-in keys, two authorization states),
   two Fastboot reboot-decision cases, and continue/boot callback state cases.
   The boot callback is also a control against earlier work, not entirely
   independent new behavior. Callback execution, persistent storage, and
   hardware reset are replaced by mock guest-memory instrumentation.
   VM failures are counted as failures, never silently discarded.

3. Offline SHA-verified reuse of V17 updater/FDT results, without
   re-running the same ELF decompilation. New conservative address-interval
   overlap calculations explicitly remain hypothetical until runtime RAM
   relocations and device reservations are proven.

4. Preserve all 85 original research question identifiers and associate
   new evidence with the appropriate questions without counting name
   matches or unrelated static references as answers.

The previous 1,239 V17 guest cases are not repeated. All proprietary
firmware bytes are confined to ephemeral private guest fixture files and
are NOT included in the final ZIP.

## Important V18 files

- V18-READ-ME-FIRST.md
- V18-STAGE-MANIFEST.json
- V18-REPAIRED-DATAFLOW-AND-85-QUESTIONS.json
- V18-PRIOR-FDT-AND-UPDATER-TRIANGULATION.json
- deep-p1/V17-p1-INDIRECT-AND-MMIO.jsonl (previously empty)
- deep-p3/V17-p3-INDIRECT-AND-MMIO.jsonl (previously empty)
- deep-p1/V17-p1-HIGH-PCODE-FUNCTIONS.jsonl
- deep-p3/V17-p3-HIGH-PCODE-FUNCTIONS.jsonl
- V18-ACTUAL-VENDOR-ARM-CASES.json
- v18-arm-vendor-console.log
- ZIP_INDEX.json (SHA256 verification)

Original owner source bytes and updater ELFs are not placed in ZIP.

## Limits and verification

There is no physical MPC access, no eMMC or USB passthrough, no firmware
writes, no Fastboot authorization command sent to hardware. The Ghidra
projects are -readOnly, QEMU uses -nic none and -snapshot, and storage is
disposable. The generic ARM virt VM is not an RK3288 BootROM simulator.

This branch is committed and has source-level regressions and a preflight
command, but has NOT yet run on the owner's actual Ghidra/QEMU environment.
Do not claim success of V18 without the actual returned ZIP and execution
evidence.
