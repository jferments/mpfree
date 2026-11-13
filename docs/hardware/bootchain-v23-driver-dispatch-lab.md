# MPFree V23 — Driver-table resolution, first-stage boot and owner SD/eMMC mapping

Branch: research/bootchain-v23-driver-dispatch-20261113

## Verified owner V22 evidence

The owner V22 ZIP bootchain-wide-20261113T003133Z-v22-181363.zip
contains nine SHA-verified reports; ZIP CRC valid. Actual runtime
15.52 seconds. SHA-pinned owner P3 Ghidra decompiled 531/531 selected
functions, with 1,815 STORE and 122 CALLIND dependencies. Real U-Boot
ARM virt guest returned 29/29 passing cases: 12 original ARM division,
8 original storage-wrapper/nonwriting callback, 9 original registered
write-handler/low-level-transport-stub cases. No physical media writes.

Further verified V21 function-pointer inventory and V22 pseudocode
resolve the immediate hardware-side command callback:

- FUN_0011D8E8 registers the block-device interface.
- FUN_0011DE50 is the registered write handler.
- FUN_0011CA78 tail-dispatches to the backend driver function table.
- FUN_0011C964 installs the driver-table pointer 0x0012F8F4.
- Owner table at 0x0012F8F4: pointer 0x0011C4E9, to FUN_0011C4E8.
- Owner table at 0x0012F8F8: pointer 0x0011C913, to FUN_0011C912.
- FUN_0011C4E8 issues/polls commands through controller registers.
- FUN_0011C912 configures controller registers.

This is source proof of the controller driver callback, not proof
which live controller instance U-Boot numbers as SD versus eMMC.

## One-command ML-tower invocation

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-v23-driver-dispatch-20261113 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_v23.py" \
  "$R/tools/mpc-one/test_v23.py" \
  "$R/tools/ghidra/MPFreeV23DriverCommand.py" && \
arm-linux-gnueabihf-gcc \
  -std=c11 -O2 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/vendor_probe_v23.c" && \
python3 "$R/tools/mpc-one/test_v23.py" && \
python3 "$R/tools/mpc-one/run_v23.py"
~~~

The runner autodetects newest SHA-indexed private V22 and V17 ZIPs under
~/mpfree-work. Overrides: --prior-v22 /absolute/v22.zip and
--prior-v17 /absolute/v17.zip.

One private output ZIP:
~/mpfree-work/bootchain-wide-UTC-v23-PID.zip
Contains only indexed research text/JSON/console evidence, no firmware.

## Concurrent research stages

1. P1 SPL Ghidra: verified firmware source, recursive first-stage
   entry, media selector, USB recovery and loader-handoff call graph,
   four-hop typed High P-code STORE/CALLIND/BRANCHIND witnesses.

2. P3 U-Boot Ghidra: verified firmware source, actual 0x0012F8F4
   driver table, genuine controller-command and config routines,
   backend registration/transport, removable boot and bootm call
   chains; source-level typed address/value dependencies and
   per-instruction witnesses.

3. Real owner ARM virt: 60 NEW independent guest scenarios:
   16 pass through the original FUN_0011CA78 and genuine firmware
   table to the intercepted controller-command destination;
   32 pass through FUN_0011CA78 with synthetic driver table and
   instrumented nonwriting callback to measure argument/return ABI;
   12 execute genuine FUN_0011C912 on synthetic controller registers,
   verifying configuration and command-register canaries.
   Hardware-command FUN_0011C4E8 body is NEVER executed; intercepted
   before its MMIO effects. No host storage device exposed.

4. Verified original V17 owner ACVA/ACVA-C FDT and boot-script
   crosswalk. In both DTBs Linux alias mmc0 points to
   0xff0f0000 (nonremovable 8-bit eMMC), while mmc1 points
   to 0xff0c0000 (4-bit removable SD with card-detect GPIO).
   Other two controllers are disabled. Original boot script
   passes CRC checks, loads zImage and DTB from currently selected
   devtype/devnum, then uses bootz. It uses setenv bootargs but
   contains no saveenv. This does NOT prove U-Boot mmc indices equal
   Linux DT aliases or that SD boot succeeds.

5. Every original 85-question ID is retained. New evidence is
   categorized as static original source, previous verified FDT,
   or isolated real ARM guest. No automatic closure based solely
   on function-name matches or a test count.

## Device safety and proof limits

No physical MPC, MMC/eMMC/USB passthrough, MaskROM operation,
eFuse/OTP write, persistent U-Boot environment change, flash/erase,
vendor updater or Linux kernel handoff. Generic ARM virt is NOT an
RK3288 emulator. Source firmware copies are private and temporary,
then deleted. All output ZIP files are per-member SHA-256 checked.

Source and host regression tests are committed, but the actual
V23 Ghidra and QEMU work has not run yet on the owner's ML-tower.
Report run failures as failures; do not infer actual U-Boot SD
device index, physical MMC I/O or nonpersistent boot from this run.
