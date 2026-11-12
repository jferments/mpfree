# MPFree V15 — directed Fastboot/direct-boot investigation

Branch: research/bootchain-directed-v15-20261112
Date: 2026-11-12
Status: owner-private, offline research; NO physical MPC, USB, eMMC or OTP access.

## One command on ML-tower

Use the existing *separate* MPFree boot-audit working tree:

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-directed-v15-20261112 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_bootchain_directed_v15.py" \
  "$R/tools/mpc-one/tests/test_bootchain_directed_v15.py" \
  "$R/tools/ghidra/MPFreeDirectedBootV15.py" && \
arm-linux-gnueabihf-gcc -std=c11 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/v15_vendor_permission_guest.c" && \
python3 -m unittest discover \
  -s "$R/tools/mpc-one/tests" \
  -p 'test_bootchain_directed_v15.py' -v && \
python3 "$R/tools/mpc-one/run_bootchain_directed_v15.py"
~~~

The runner auto-detects the newest
~/mpfree-work/bootchain-wide-*-v14-*.zip and verifies every indexed member
against its SHA-256 before using the prior research. Override with
--prior-v14 /absolute/path/to/private-v14.zip.

**Upload only the ONE resulting**
~/mpfree-work/bootchain-wide-<UTC>-v15-<PID>.zip.
The script writes a checksum sidecar separately. Never publish the ZIP or
derived proprietary Ghidra reports to GitHub.

## What is new compared with V14

1. SHA-pinned **directed Ghidra P3 U-Boot**: revisit the actual OEM command,
   permission flag, download, USB receive, RAM bootm, image CRC, U-Boot
   command interpreter, getvar, board/SD selection and recovery functions.
   Capture bounded Ghidra high P-code for LOAD, STORE, CALL/IND, branching,
   and comparisons, plus instruction windows, xrefs and literal pointer cells.

2. SHA-pinned **directed Ghidra P1 SPL**: investigate ROM entry, SPL boot-device
   order, MMC device, image loader, recovery and handoff. No re-decompilation
   of every recognized V14 function; expand callers and callees of seeds.

3. **NEW: execute actual vendor instruction bytes in isolated QEMU ARM guest.**
   The specific hash-verified P3 page 0x00122000..0x00122fff is exported to
   a private temporary file, then copied into a QEMU fixture. Inside the
   guest, ONLY the genuine Thumb function at 0x00122834 is invoked.
   Guest memory for authorization global 0x0014d690 is synthetic.
   Its denied-path USB-response function is replaced in guest memory with
   Thumb BX LR, so no vendor USB/peripheral functions execute. The test
   exercises flag values 0/1/2 and checks returns and unmodified state.
   It does NOT execute the OEM unlock command, U-Boot bootm, full vendor
   Fastboot, updater, hardware, flash or MaskROM.

4. **Directed V14 graph reuse**: verify the V14 owner input checksums; trace
   actual paths among OEM, permission, run_command, image loader, bootm,
   reset and candidate environment writers. Examine the full 1050-function
   P3 call graph as saved in V14, not simply a few string matches. Record
   potentially mutating call paths without claiming absence of indirect
   calls as proof of safety.

5. **Updater backend reuse**: parse the V11 updater Ghidra function graph
   carried in the verified V14 ZIP; identify candidate write, verification,
   bootloader, signature and indirect callback routes. Do not re-run the
   same V11 Ghidra analyses and count duplicates as new progress.

6. **85-question inventory**: assign every question the closest previously
   identified candidate functions and its original classification.
   Keyword matches are labeled prior-corpus indexing ONLY; they never
   automatically mark a question answered or inflate the progress totals.

Stages 1, 2 and prior-graph work begin concurrently. ARM QEMU runs when
the SHA-verified P3 temporary slice is available. A missing Ghidra/VM
dependency yields an explicit stage error, with other stages preserved.

## Prerequisites / input safety

Reuses the same saved analyzed Ghidra P1/P3 projects and ARM QEMU kernel,
cloned rootfs, overlay and disposable snapshot setup from V14.
Requires arm-linux-gnueabihf-gcc and mkfs.ext4 for the guest fixture.
No packages are auto-installed. No host live /dev/mmc or USB device is
opened. No real vendor updater or owner rootfs maintenance utility executes.

Only the private verified 4KB P3 page is materialized temporarily under
~/mpfree-work/mpfree-v15-private-fn-* and deleted after the run. It is
**NOT** included in the final ZIP. The C test binary and synthetic fixture
ext4 image are also deleted. The VM uses -snapshot and -nic none.

## Single ZIP results

- V15-READ-ME-FIRST.md: measured stage results and evidence limitations.
- V15-STAGE-MANIFEST.json: exit statuses and real timings.
- V15-EXACT-FINDINGS.json: statically established, isolated ARM and open facts.
- V15-PRIOR-GRAPH-AND-85-QUESTIONS.json: paths and question-by-question
  candidates from the SHA-verified V14/V11 corpus, clearly marked prior.
- v15-p3-directed.json: *new* owner-P3 Ghidra P-code, direct/indirect calls,
  literal values, original command dispatch, xrefs and permission alias.
- v15-p1-directed.json: *new* early SPL focused boot-source/recovery analysis.
- V15-ACTUAL-VENDOR-FUNCTION-VM.json: mock-RAM results from the **real** code
  slice, not an independently written Fastboot behavior model.
- v15-qemu-vendor-permission-console.log: serial transcript.
- ZIP_INDEX.json: per-file SHA-256, sizes and verified ZIP contents.

## Interpretation

Passing the isolated ARM experiment confirms only the specific permission
function's behavior under controlled mock-memory conditions. It does not
demonstrate that any physical MPC One will accept the public OEM token, that
unlocking is persistent, that any arbitrary kernel will boot, or that the
factory bootloader is independently recoverable. Replacing P6 or changing
P1/P2/P3 remains outside the research scope.

Never run fastboot flash, erase, OEM unlock, U-Boot saveenv, RockUSB
programming, OTP/eFuse operations or MaskROM experiments as part of V15.
