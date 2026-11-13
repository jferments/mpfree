# MPFree V25 — Native MMC handler, unrecognized Thumb command handlers and SPL media order

Research branch: research/bootchain-v25-native-mmc-handlers-20261113

## Verified input: V24 actual observations

Owner's ZIP bootchain-wide-20261113T062738Z-v24-337963.zip passed CRC and all 15 per-file SHA-256 checks.

V24 actually ran for 13.61 seconds:
- P1 Ghidra decompiled 147/147 targeted functions.
- P3 Ghidra decompiled 429/429 targeted functions but its stage failed.
- Independent source-byte decoder validated 86 real U-Boot 28-byte command descriptors with zero malformed records; 37 allowlisted command names.
- Original U-Boot command lookup returned exact matching native command descriptors for 32/32 guest cases.
- Original MMC selection returned correct behavior in 16/16 cases, but the V24 harness marked 4 absent IDs (2,3) failed because it wrongly expected one initialization call instead of zero.
- The V24 ARM status remains 44/48, as officially reported; do not rewrite those results. Source diagnosis establishes four expectation defects.

The saved Ghidra project recognizes only 25 of 86 native handler addresses as existing functions. The 61 other handler pointers are present in pinned source and were confirmed by the successful native ARM lookup. V24 equated missing Ghidra Function objects with invalid descriptors; independent byte-decoder disproved that interpretation.

Owner-compiled default boot_targets has tokens mmc0 mmc1. It is not proof that the active saved U-Boot environment uses these defaults or that Linux MMC aliases equal U-Boot numbering.

## One copy-and-paste command (ML-tower)

    R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
    git -C "$R" fetch origin research/bootchain-v25-native-mmc-handlers-20261113 && \
    git -C "$R" switch --detach FETCH_HEAD && \
    python3 -m py_compile \
      "$R/tools/mpc-one/run_v25.py" \
      "$R/tools/mpc-one/test_v25.py" \
      "$R/tools/ghidra/MPFreeV25CommandRecovery.py" \
      "$R/tools/ghidra/MPFreeV25SPLBootOrder.py" && \
    arm-linux-gnueabihf-gcc \
      -std=c11 -O2 -Wall -Wextra -Werror -fsyntax-only \
      "$R/tools/mpc-one/vendor_probe_v25.c" && \
    python3 "$R/tools/mpc-one/test_v25.py" && \
    python3 "$R/tools/mpc-one/run_v25.py"

The runner discovers the newest private SHA-indexed V24 archive from ~/mpfree-work. To override it, use --prior-v24 /abs/private-v24.zip.

The output is ONE private SHA-indexed research ZIP:
  ~/mpfree-work/bootchain-wide-<UTC>-v25-<PID>.zip

Raw proprietary firmware bytes are never packaged or committed.

## Four genuinely new concurrent investigations

1. P1 SPL original 32 KiB owner image, SHA pinned, saved Ghidra project read only:
   Decode the actual five source priority-list entries using literal cell
   0xff7053fc and priority array at 0xff709c70. Decode category/handler
   descriptor table from 0xff705404, cross-check five-iteration selector
   FUN_ff705350 and sentinel 0xDEADBEEF. Decompile all 213 recognized P1
   functions with direct graph and typed STORE/CALLIND source evidence.
   These are new actual values, not only a previously extracted pointer.

2. P3 U-Boot 86 native registered command handlers:
   From 86 seven-word records at 0x0014464c, recover every source Thumb
   handler pointer and execute Ghidra PseudoDisassembler with explicit
   Thumb TMode on each entry in the saved Ghidra project WITHOUT writing
   or saving the project. Trace up to 400 reachable instruction sites
   for every handler, including 61 previously unrecognized functions.
   Record branch targets, fallthroughs, source instruction text and
   failure sites rather than treating an absent Function object as
   invalid code. In parallel, deeply decompile recognized caller/callee
   closures around MMC native command handler, numeric selection,
   read-only filesystem loading, bootz, environment editing and saveenv.

3. Independent original source-byte crosswalk:
   Host Python decodes the same SHA-pinned 86 command descriptors without
   relying on Ghidra function objects. Verifies native MMC handler
   pointer 0x00104f11 and owner default boot_targets=mmc0 mmc1;
   hashes potentially private bootcmd values rather than printing them.
   Documents V24 four false-negative assertions independently.

4. Isolated real U-Boot native MMC command handler runtime:
   The guest executes *actual registered* Thumb handler 0x00104f11
   with original cmd descriptor 0x00144ca4, rather than V24's command
   descriptor lookup. 20 new guest processes exercise harmless native
   mmc list, dev 0/1/missing, info, part, rescan, partition arguments,
   and invalid syntax. Synthetic registered backend objects replace
   physical devices. Before any native handler is called, the guest
   intercepts the real driver command gateway 0x0011ca78, block-write
   callback 0x0011de50, erase 0x0011dd4c, saveenv 0x00108110,
   hardware reset 0x0011622a and low-level initialization 0x0011daf4.
   Instrumented counters and owner global-cell canaries ensure that a
   completed test did not reach blocked persistent functionality.
   Crash, timeout or unexpected effects are FAILURE, not an answer.

The original 85 questions are preserved and annotated only with
specific V25 source and runtime evidence, without invented physical
behavior or question closures.

## Safety and proof limits

The original owner firmware and saved Ghidra projects are hash-verified
and read-only. QEMU uses disposable generic ARM virt with no host USB,
MMC, eMMC, network or other physical passthrough. It is not a model
of Rockchip RK3288 BootROM or the physical SD card. No real saveenv,
flash, erase, reset, U-Boot external boot or kernel/FDT handoff occurs.

The source and host preflight tests are committed, but the owner-side
full Ghidra/QEMU execution has not yet been verified. The copy/paste
command runs syntax, ARM compiler warnings-as-errors, and host tests
before launching research. Never infer success from runtime duration.
