# MPFree V24 — Enumerate the real U-Boot commands and test MMC device IDs safely

Branch: research/bootchain-v24-uboot-command-registry-20261113

## Why V24, specifically

The owner's newest private archive
bootchain-wide-20261113T054852Z-v23-299625.zip was independently
verified: ZIP CRC passed, 14/14 SHA-indexed reports verified.
All four stages passed in 15.67 seconds:
- P1 Ghidra: 147/147 selected functions, 837 typed STORE and 29 CALLIND.
- P3 Ghidra: 547/547 selected functions, 1842 STORE and 126 CALLIND.
- 60/60 actual vendor ARM guest tests passed: 16 direct original driver
  table->intercepted controller, 32 driver-dispatch callback-ABI tests,
  12 real controller-config-with-fake-register cases.
- Original V17 DTB crosswalk passed: eMMC at 0xff0f0000 and removable SD
  at 0xff0c0000, under Linux aliases mmc0/mmc1 respectively.
  U-Boot's MMC device numbering is NOT yet verified.

V23 completed the lower-level driver call-chain. Continuing to repeat
the 16 driver callbacks or 12 fake register settings will not advance
the user goal of read-only direct boot from SD.

Cross-reading *complete V18* decompilation identifies the NEW
high-value command registry:

- FUN_00110CCC at 0x00110CCC examines registered native U-Boot
  command descriptors, advancing in units of seven 32-bit words.
- FUN_00110D2C is the wrapper that obtains the table start/end
  from owner literal cells 0x00110D3C/0x00110D40 and computes count.
- FUN_001111E0 validates argument limits and calls the command
  handler found by that lookup. DO NOT execute this with unknown input
  or a potentially destructive command.
- FUN_00110484 is the command-parser entry.
- FUN_00104F88 requests a numbered MMC device by invoking the
  actual registered backend lookup FUN_0011C3AC; initializes it
  through FUN_0011DAF4. This can be safely tested with synthetic
  registry entries and init stub.
- FUN_00105620 prints MMC details, but that alone does not prove
  physical SD numbering or safe filesystem loading.

Importantly, prior Ghidra roots FUN_00116F64, FUN_00116EC8,
FUN_00116C8C are DFU/updater MMC functions, not evidence that stock
U-Boot autoboot enumerates the removable SD card. V24 explicitly
distinguishes those domains.

## Single copy-paste command on the ML-tower

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-v24-uboot-command-registry-20261113 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_v24.py" \
  "$R/tools/mpc-one/test_v24.py" \
  "$R/tools/ghidra/MPFreeV24CommandRegistry.py" && \
arm-linux-gnueabihf-gcc \
  -std=c11 -O2 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/vendor_probe_v24.c" && \
python3 "$R/tools/mpc-one/test_v24.py" && \
python3 "$R/tools/mpc-one/run_v24.py"
~~~

Input: automatically selects the newest original SHA-indexed private
V23 owner research ZIP under ~/mpfree-work, or specify
--prior-v23 /absolute/path/to/verified-v23.zip.

Output: ONE private ZIP,
  ~/mpfree-work/bootchain-wide-<UTC>-v24-<PID>.zip
with complete member SHA-256 validation and independent checksum sidecar.

## Actual concurrent work (not synthetic prior tests)

1. Original P1/SPL Ghidra (read-only and SHA pinned):
   recursive boot-device preference table, transfer/handoff and
   storage-loader selection, typed STORE/CALLIND/BRANCHIND dependencies,
   caller/callee closures and genuine source instructions. Disting-
   uish SPL selection from U-Boot command handling.

2. Original P3/U-Boot Ghidra (read-only and SHA pinned):
   fully decode the native 7-word per-command table from the actual
   owner binary. Only explicitly allowlisted command names appear
   in report text; all other names are represented by SHA-256 hashes
   and lengths to avoid exposing proprietary unlock strings.
   For every discovered allowlisted boot/storage command, dynamically
   add its concrete registered handler to the root graph and deeply
   decompile the handler and nearby function paths. Investigate MMC
   device-ID lookup, bootz/source/load command handlers, boot env,
   SD filesystem and alternate DFU updater routes.

3. A SECOND, independently implemented host Python decoder of the
   original SHA-pinned P3 image, running concurrently with Ghidra.
   It validates table boundaries and 28-byte stride, independently
   enumerates native command handlers, verifies names/argument counts
   and reports whitelisted boot-target strings and exact occurrences
   of eMMC/SD physical controller base words without claiming actual
   U-Boot device numbering. This checks that Ghidra and the VM have
   not merely repeated the same mistaken interpretation.

4. NEW actual owner U-Boot command lookup in generic ARM QEMU:
   32 distinct safe name/prefix queries invoke genuine FUN_00110CCC
   and FUN_00110D2C. These functions return native command descriptor
   pointers; independent raw source-byte parsing checks those results.
   THE COMMAND HANDLERS THEMSELVES ARE NEVER CALLED. No saveenv,
   mmc write, bootz, reset, flash or other command is executed.

5. NEW actual owner MMC device-index lookup in generic ARM QEMU:
   16 synthetic-list scenarios invoke original FUN_00104F88 with
   fake registered MMC IDs, missing IDs, forced reinitialization and
   mocked initialization errors. FUN_0011C3AC traversal executes;
   physical initialization FUN_0011DAF4 is intercepted.
   This proves the numbering->registry lookup semantics, NOT that a
   physical U-Boot index equals Linux /aliases or that SD boot works.

6. Carry all original 85 boot/recovery questions without auto-closure
   from a registered command name or synthetic device ID.

## Safety and evidence qualifications

No physical MPC contact. No host USB/eMMC/SD passed into QEMU. The
generic ARM virt guest executes only owner lookup and safe selection
functions against fake registered device memory; it NEVER dispatches
the found U-Boot commands, invokes the native block-write callbacks,
enters actual MMC controller MMIO, modifies firmware, runs saveenv
or bootz, or changes hardware boot priority. Raw proprietary P3 bytes
are held in a private short-lived directory and never included in ZIP.

The branch sources and regression tests are committed. Their
end-to-end execution on the user's private Ghidra/QEMU environment
has NOT yet occurred. Syntax, ARM C compilation and host regression
tests run before the full lab. An invalid command-table layout, a
missing firmware SHA or a guest failure is recorded as failure, not
misrepresented as a successful hardware test.
