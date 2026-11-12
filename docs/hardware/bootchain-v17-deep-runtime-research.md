# MPFree V17 — Full fixed-point Ghidra + actual vendor ARM-QEMU research

**Branch:** research/bootchain-deep-runtime-v17-20261112

V17 is a substantial new research pass rather than a 10-second summary of existing reports. It starts new analysis of all recognized P1/P3 functions and both updater ELF programs, traverses caller and callee closures to a fixed point, captures full High P-code evidence, boots the generic ARM guest, and executes >1,100 distinct real vendor function inputs with hardware/side-effect callbacks mocked. No original MPC device, eMMC, physical USB, or other owner firmware is modified.

## One copy/paste command on ML-tower

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-deep-runtime-v17-20261112 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_v17.py" \
  "$R/tools/mpc-one/offline_boot_assets_v17.py" \
  "$R/tools/mpc-one/test_v17.py" \
  "$R/tools/ghidra/MPFreeV17FixedPointDive.py" \
  "$R/tools/ghidra/MPFreeV17UpdaterDive.py" && \
arm-linux-gnueabihf-gcc -std=c11 -O2 -Wall -Wextra -Werror \
  -fsyntax-only "$R/tools/mpc-one/vendor_probe_v17.c" && \
python3 "$R/tools/mpc-one/test_v17.py" && \
python3 "$R/tools/mpc-one/run_v17.py"
~~~

The runner finds the latest previously verified V16 research ZIP in
~/mpfree-work; --prior-v16 allows specifying an explicit one.

## Six major investigations

**Full P1 SPL Ghidra:** Verify known P1 image hash, enumerate all
Ghidra-recognized functions, recurse to a fixed point in the complete
direct caller/callee graph from ROM loader, boot media, SPL handoff,
DDR, PMU and recovery roots. Decompile all recognized functions and
inspect every High P-code operation for relevant LOAD, STORE, CALL,
CALLIND, branch, comparison, PHI/MULTIEQUAL and pointer operations.
Export function-level dataflow and unaccounted indirect calls to
owner-private JSONL. No arbitrary two-hop or four-hop stop.

**Full P3 U-Boot Ghidra:** Same full pass, domains covering OEM authorization,
Fastboot download/USB receive, flash and erase, bootm/bootz/FDT/image
verification, command interpreter, SD/eMMC selection, persistent environment
and recovery. Identify candidate function-pointer tables and defined
string references. Export true per-function evidence and exact counts
for function recognition, decompilation, P-code and indirect branch sites.

**Fresh updater Ghidra:** Re-import SHA-verified owner ELF copies of
az01-update and libaz01-prog into separate temporary Ghidra projects.
Perform complete decompilation and fixed-point signature, backend
dispatch, RockUSB/Fastboot transport, partition write/commit and
recovery graph analyses, including new callback-table discovery.
No updater ELF is executed or copied into the ZIP.

**Actual owner ARM code in QEMU:** The P3 Ghidra pass exports the SHA-pinned
raw P3 byte image immediately after verification, then continues
full High P-code analysis while QEMU starts. QEMU boots the previously
validated generic ARM Linux-virt guest, using a private disposable ext4
fixture, -snapshot, no network, no monitor and no USB-host passthrough.
The guest executes *actual vendor ARM functions*, not synthetic
Fastboot model functions, with command/storage/bootm/reset/USB side-effect
targets replaced by instrumented guest-memory return trampolines.

New real-code scenarios:
- Locked vs unlocked OEM shell/command-dispatch branches, with call counts.
- Locked vs unlocked OEM environment/persistence branches, with storage
  invocation replaced before it can execute.
- Boot-completion path invocation of image loader, bootm and reset (mocked).
- Download size parser: 0, max, overflow and 128 deterministic internal values.
- USB receive copies: >1,000 size cross-products and deterministic interior
  pairs, with canaries and byte-for-byte copy/count validation.

Every case runs as a separate ARM guest process with an ACK and timeout;
a crash or failure is not counted as a successful test. These cases are
new controlled inputs, not 1,100 separate research questions answered.

**Owner /boot and device trees:** Read-only debugfs accesses real copied ext4
rootfs boot assets. Completely parse (not just selected properties) both
ACVA and ACVA-C FDT structures, record all node/property differences,
decode the actual stock U-Boot script, verify its CRCs, identify Linux
zImage header, and flag *candidate* memory interval crossings.

**85-question traceable ledger:** Preserve the original question IDs and
classify each new relevant static/VM evidence source, with exact counts
and uncertainty. Never auto-close questions from keyword matches or
claim RK3288 hardware/MaskROM recovery from generic ARM emulation.

## Output

ONE PRIVATE ZIP:
~/mpfree-work/bootchain-wide-<UTC>-v17-<PID>.zip

Its SHA-256 is printed and written to a sidecar. It includes V17 manifest,
new 85-question evidence ledger, full P1/P3 JSONL High P-code, direct and
indirect fixed-point graphs, fresh updater ELF JSONL graphs, actual QEMU
per-case observations and serial transcript, complete owner DTB diff and
memory-boundary hypotheses, and per-file SHA-256 ZIP_INDEX.json.

**Do not commit this private ZIP or decompiled code reports to GitHub.**
Vendor firmware bytes and temporary imported updater ELF files remain
outside the ZIP, under a temporary owner-private directory that is
deleted at completion.

## Limitations

This lab is substantially broader, but it is still not an RK3288 silicon
BootROM or complete U-Boot emulator. Hardware-only SD selection, physical
DDR reservations, eFuse policy and independent rescue capability are not
claimed as proven. No Fastboot unlock, flash/erase, saveenv, eMMC programming
or hardware restore is attempted. Extensive data gathering is real;
question resolution must still follow the evidence.
