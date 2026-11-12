# MPFree V13: Closing Fastboot authorization and direct-boot questions

Date: 2026-11-12. Branch: research/fastboot-directboot-v13-20261112.
Read-only offline testing only. No MPC SSH, USB, eMMC, vendor executable,
Fastboot, loader, ROM, OTP, environment or firmware modification.

## Why V13 is different

V12 found the exact OEM unlock marker at 0x00132bab (reference from
0x00122892) and decompiled the handler at 0x00122854. That handler stores 1
through a pointer loaded from literal cell 0x00122928. The Fastboot
permission function at 0x00122834 reads through a pointer in 0x0012284c.
V12 failed to extract and compare the two pointer values, and its question
matrix called everything a "lead" even where a direct call edge was proven.

V13 reads the verified raw literal cells directly, resolves pointer aliases,
reconstructs the eight-record Fastboot command dispatch table, follows
download/flash/erase/bootm calls, checks candidate bootcount/reboot registers,
and carries forward the V12 question matrix with explicit static evidence
versus hardware gaps.

## One terminal command on ML-tower

Use the existing separate research working tree:

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/fastboot-directboot-v13-20261112 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_fastboot_directboot_v13.py" \
  "$R/tools/mpc-one/run_fastboot_directboot_v12.py" \
  "$R/tools/mpc-one/offline_fastboot_boot_assets_v13.py" \
  "$R/tools/mpc-one/tests/test_fastboot_directboot_v13.py" \
  "$R/tools/mpc-one/tests/test_fastboot_boot_assets_v13.py" && \
python3 -m unittest discover \
  -s "$R/tools/mpc-one/tests" \
  -p 'test_*v13.py' -v && \
python3 "$R/tools/mpc-one/run_fastboot_directboot_v13.py"
~~~

The runner automatically locates the newest existing private V12 ZIP in
~/mpfree-work/ and SHA-checks every indexed member before importing only
the question text. Override with --prior-v12 PATH for a specific archive.

Final output: one private, SHA-indexed
~/mpfree-work/bootchain-wide-<UTC>-v13-<PID>.zip plus checksum sidecar.
Upload the ZIP only, not each private report and not firmware.

## Concurrent stages

- SHA-pinned P3 saved U-Boot project in Ghidra -readOnly -noanalysis.
  Extracts exact pointer cells for the OEM unlock writer and permission gate,
  pointer alias, dispatcher records, RAM download addresses and callback
  targets; decompiles selected ownership and dispatch functions.
- SHA-pinned P1 saved SPL project in Ghidra -readOnly -noanalysis,
  traces boot selection, ROM return, update/recovery and handoff.
- ARM QEMU virt boots existing 3.10 cloned rootfs in snapshot mode with
  no network, USB or physical devices and executes original synthetic C model.
  This reruns a regression baseline; it is NOT vendor U-Boot execution.
- Native model reuses the V12 synthetic ARM policy specification.
- Offline rootfs and backup inspection reuses verified V12 routines.
- Additional offline /boot analysis checks legacy U-Boot script header and payload CRCs,
  ARM zImage magic, both ACVA DTB structures, reboot-mode properties and
  selected differences. It only reads original regular-file ext4 with debugfs.

The five independent jobs run concurrently and write failure states rather
than reporting a partial run as complete.

## V13 focal answers and evidence thresholds

1. **OEM permission pointer alias:** two literal-cell values must be the
   same nonzero address. If true, it establishes a static state link between
   the OEM handler and the Fastboot permission check.
2. **Eight-entry Fastboot dispatch:** extract eight command-handler pairs
   from the pinned raw P3 image; do not infer boot/flash permissions from
   command names alone.
3. **Download, erase and flash:** require explicit Ghidra direct call edges
   to permission helper 0x00122834.
4. **RAM boot:** require explicit Fastboot completion to bootm call edge.
   This does NOT prove compatibility with raw zImage or working FDT.
5. **Boot script:** verify actual /boot script metadata, kernel and DTBs
   from an existing regular-file rootfs.
6. **PMU bootcount:** search register and magic; missing raw literals
   cannot establish the absence of the runtime mechanism.
7. **Memory and rescue:** describe address candidates but do not label
   an advertised image size as safe DRAM; actual owner-independent
   MaskROM recovery remains a hardware-only proof requirement.

### ZIP reports

V13-READ-ME-FIRST.md, V13-STAGE-MANIFEST.json,
V13-EXACT-FINDINGS.json, V13-RESEARCH-QUESTION-MATRIX.json,
V13-PRIOR-QUESTION-LINEAGE.json, offline-boot-assets-v13.json,
ghidra-p3-v13.json,
ghidra-p1-v13.json, qemu-v12-console.log, qemu-v12-stage.json,
native-synthetic-results.log, offline-rootfs-and-boot-map-v12.json
and SHA-indexed manifest.

### Cautions

Only Ghidra projects and **regular files** are analyzed. The original
vendor binaries are never executed by QEMU or on the device. QEMU ARM virt
is not an RK3288 SoC/ROM emulator; model tests are demonstrations of our
policy, not observed Akai Fastboot behavior.

The OEM command is a state-changing command. Do NOT use this run as approval
to send it to a real MPC. No flash, erase, eFuse, resetenv, bootloader writes,
USB passthrough or SD-first environment persistence are performed.
