# MPFree V26 — Resolve UART-faulted native MMC handler and dynamic SPL boot-source value

Branch: research/bootchain-v26-mmc-uart-and-spl-source-20261113

## Source evidence: V25 user-uploaded archive

bootchain-wide-20261113T075528Z-v25-397105.zip was verified:
ZIP CRC passes; all **15** indexed research files match SHA-256.

Actual V25 execution time: **10.16 seconds**.
- Full P1 recognized function decompilation 213/213.
- Original P3 86/86 native command entrypoints produced some Ghidra
  pseudo-instructions, totalling 4,414 instruction records.
- 61 handlers were not recognized as functions in saved Ghidra project.
- 6 pseudo-handler traces encountered decode errors. A successful
  entry-point disassembly is NOT complete source recovery.
- 238 recognized source functions decompiled around MMC/boot/env,
  with 3,441 High P-code operation witnesses.
- Independent original boot-target decoder passed.
- Actual ARM guest stage: **2/20 passed**. The two successes were
  invalid-subcommand returns (-1); **no successful mmc list/dev/info
  operation has yet been validated**.

The remaining **18** command cases all signalled SIGSEGV:
- Fault PC = **0x0011F592**
- Link register = **0x0010D16D**
- Phase = 41 (real native MMC handler)
- Original FUN_0011F580 at 0x0011F580 is UART transmit; it loops
  polling controller status through a hardware MMIO pointer, then
  writes a transmit register. The generic ARM virt VM does not map
  the MPC's UART. The 18 crashes are VM fixture incompleteness,
  NOT evidence of a real MPC firmware defect.

Original P1 priority array has FIVE words, all **0xDEADBEEF**.
This is NOT an initialized boot-device order. Before looping,
FUN_ff705350 invokes FUN_ff705342; the latter writes the actual
return from **FUN_ff704120** into the first priority slot.
The complete V25 source decompilation reveals the returned value is
**1** in the default/error path, or **8** if
FUN_ff705b16(local_c) returns **0x35**.
We have NOT yet identified what that condition means physically
or which controller corresponds to values 1 or 8.

## Single-copy-paste run command

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-v26-mmc-uart-and-spl-source-20261113 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_v26.py" \
  "$R/tools/mpc-one/test_v26.py" \
  "$R/tools/ghidra/MPFreeV26MMCRootAndUART.py" \
  "$R/tools/ghidra/MPFreeV26SPLDynamicMedia.py" && \
arm-linux-gnueabihf-gcc \
  -std=c11 -O2 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/vendor_probe_v26.c" && \
python3 "$R/tools/mpc-one/test_v26.py" && \
python3 "$R/tools/mpc-one/run_v26.py"
~~~

Runner auto-discovers latest SHA-indexed owner-private V25 ZIP under
~/mpfree-work. Pass --prior-v25 /absolute/path.zip to override.

One private SHA-indexed output ZIP:

~/mpfree-work/bootchain-wide-<UTC>-v26-<PID>.zip

No original proprietary firmware bytes are archived, shared or
committed. Owner P1/P3 and saved Ghidra projects are read-only.

## Actual new investigations

### A. P1 SPL runtime source and special-mode chain

Read-only SHA-pinned P1 project. Decompile *all* 213 recognized source
functions again for the original caller/callee graph, and deeply trace
the exact new roots:
- FUN_ff704120: real source of media selection value 1 or 8
- FUN_ff705342: writes first dynamically selected priority slot
- FUN_ff705350: source selector loop
- FUN_ff705b16: computes value compared with 0x35
- FUN_ff705af2: preceding transformation/validation
- FUN_ff7089fc / FUN_ff708770: upstream raw boot-state acquisition

New V26-P1-DYNAMIC-BOOT-SOURCE-ORIGIN.json includes complete
pseudocode, decoded instruction sequences, all High P-code operations
and original source addresses for these functions. The initialized
five-slot array is explicitly labelled *uninitialized sentinel*.

### B. Native U-Boot MMC subcommand table and handlers

The owner registered top-level "mmc" handler at Thumb 0x00104F11,
and its own Thumb instructions show:
- MOVS r2,#10 at 0x00104F1C: ten subcommands
- LDR r1,[0x00104F7C] at 0x00104F1E: pointer to nested table
- BL FUN_00110CCC at 0x00104F26: actual lookup

V26 Ghidra reads all **10 seven-word subcommand records** from the
actual SHA-pinned owner P3 image, decodes safe allowlisted names and
their original handler pointers. Dangerous names may be enumerated
but NONE is ever executed by the Ghidra stage.
It traces source High P-code for command selection, UART formatting,
MMC initialization, filesystem loading and boot handoff.

An independent host byte decoder reconstructs the same 10 records
without using Ghidra's saved function definitions, preventing a
repeat of V24's mistaken "valid pointer must be recognized function"
assumption. Disagreements/failures are retained, not disguised.

### C. Actual native MMC command runtime with UART fixture repair

20 controlled reruns of native "mmc" command handler inputs that
previously contained 18 false failures due to UART MMIO.
Only the **UART low-level transmit functions** at 0x0011F580 and
0x0011F2D6 are intercepted to avoid unmapped controller polling.
The original U-Boot MMC handler and lookup logic remain native.

Original MMC transport 0x0011CA78, direct controller 0x0011C4E8,
controller config 0x0011C912, block write 0x0011DE50, block erase
0x0011DD4C, saveenv 0x00108110 and reset 0x0011622A are also
guest-intercepted before any hardware interaction. The native handler
returns exact observed status and counter values.

An additional 12 **mmcinfo** cases execute its genuine registered
Thumb handler 0x00105621 against synthetic backend metadata,
including missing and available fake devices. The same UART/MMC,
persistence and reset guards apply.

A clean invalid-command return is **not evidence of successful device
enumeration**. Real SD/eMMC host assignment remains unverified.

### D. Evidence and archive

The report contains P1 dynamic source findings, the ten original
nested MMC descriptor entries from Ghidra and an independent parser,
UART fault diagnosis, all actual ARM case returns/crashes, stage
timings and the complete 85-question inherited inventory.
No questions are auto-closed based on test count or name matching.

## Validation and limitations

V26 code is committed but has **not** been cross-compiled against
the owner's ARM compiler or executed in the user's Ghidra/QEMU VM.
The command checks syntax, ARM compilation and host regressions
before running. A failed VM stage remains a FAILED stage.

Generic ARM virt is not Rockchip RK3288 BootROM: no real SD boot,
hardware eMMC operations, U-Boot saved environment mutation,
physical MaskROM recovery or independent Linux handoff is attempted.
