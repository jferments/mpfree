# MPFree V14: comprehensive static and evidence-based ARM VM research

Date: 2026-11-12. Branch: research/bootchain-deep-v14-20261112.
This addresses V13's main defect: **85 enumerated questions were not 85 tested questions**.
V13 took 6.6 seconds because Ghidra reopened saved projects and inspected
12 P1 + 26 P3 functions; its QEMU 42 tests repeated synthetic C tests. The
question matrix marked 0 questions answered, despite 11 exact static facts.

V14 does substantive new work on every run. It does NOT manufacture delays,
and does NOT mislabel hypotheses or synthetic results as vendor runtime proof.

## One copy/paste command (ML-tower)

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/bootchain-deep-v14-20261112 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
  "$R/tools/mpc-one/run_bootchain_deep_v14.py" \
  "$R/tools/mpc-one/run_fastboot_directboot_v13.py" \
  "$R/tools/mpc-one/run_fastboot_directboot_v12.py" \
  "$R/tools/mpc-one/offline_fastboot_boot_assets_v13.py" \
  "$R/tools/mpc-one/tests/test_bootchain_deep_v14.py" && \
arm-linux-gnueabihf-gcc -std=c11 -Wall -Wextra -Werror -fsyntax-only \
  "$R/tools/mpc-one/v14_owner_boot_fixture.c" && \
python3 -m unittest discover \
  -s "$R/tools/mpc-one/tests" \
  -p 'test_bootchain_deep_v14.py' -v && \
python3 "$R/tools/mpc-one/run_bootchain_deep_v14.py"
~~~

The script auto-selects your latest SHA-indexed V13 research ZIP and
V10 updater lineage ZIP from ~/mpfree-work. To choose inputs:

~~~bash
python3 tools/mpc-one/run_bootchain_deep_v14.py \
 --prior-v13 /absolute/path/to/v13.zip \
 --prior-v10 /absolute/path/to/v10.zip \
 --rootfs /absolute/path/to/owner-extracted-rootfs.img \
 --backup /absolute/path/to/verified-owner-emmc.img
~~~

No original files, firmware images, Ghidra projects, boot partitions, or
actual MPC devices are modified. No USB or real eMMC is exposed to QEMU.

## Concurrent research jobs (default)

**1. Full P1 SPL scan in Ghidra.** Reuses a SHA-pinned saved analyzed project
in strict read-only mode. Enumerates every function Ghidra currently knows,
builds direct call and unresolved indirect-call graphs, computes 4-hop callers
and callees of boot-media, DDR/ROM, recovery and image-loading functions,
then attempts to decompile **every recognized function**. Logs per-function
decompilation errors, number examined, exact coverage ratio, call edges,
indirect call sites and elapsed time.

**2. Full P3 U-Boot scan in Ghidra.** Same method for the larger U-Boot.
Investigates authorization/OEM dispatch, RAM boot, bootm/bootz/FDT, image
validation, eMMC/SD boot order, button ADC, reboot modes and recovery.
Captures bounded private pseudocode for contextual review, xrefs of globals,
and important literal sites. A full recognized-function pass still does not
establish runtime behavior of an unidentified or indirectly called function.

**3. Vendor updater V11 static analysis.** Reuses the earlier V11
libaz01-prog/az01-update Ghidra investigation and its verified V10 lineage.
The source ELFs are imported into *temporary* Ghidra projects and inspected,
NOT executed. Separately packages their source-verifiable reports into a
directory of the final V14 ZIP. If the verified V10 ZIP is unavailable,
this stage is explicitly marked unavailable, not counted as completed.

**4. ARM QEMU on actual owner boot-data bytes.** Boots the previously
established ARM QEMU virt guest from the cloned 3.10 rootfs under -snapshot,
with **no network, USB passthrough, monitor or host share**. Reads the
original rootfs through debugfs (no mounting original) and places its
boot.scr.uimg, two ACVA DTBs and 128-byte zImage header in a disposable
read-only virtual ext4 disk. Compiles *new original C code* for ARM and runs
it inside QEMU to check actual U-Boot script header + payload CRCs, device
tree structure, zImage magic, rejection of corrupted copies, and cross-type
format misclassification. This is meaningfully different from V12/V13's
hypothetical Fastboot policy model, but **still does not execute vendor
Fastboot or emulate RK3288 BootROM**.

**5. Independent boot-asset inspection.** Parses actual rootfs U-Boot script,
normalized executable boot commands, CRCs, ACVA/ACVA-C device-tree properties
and selected differences; records boot mode mapping and kernel header.

**6. Verified eMMC/rootfs inventory.** Checks exact owner P1 hash, extracted
ext4 rootfs, and boot assets using read-only ordinary-file reads.

All six jobs run concurrently where possible. If an individual job fails,
the rest proceed. Each job has timed stages and errors in the final archive.

## Output: ONE private ZIP to upload

Path resembles:

~/mpfree-work/bootchain-wide-20261112TxxxxxxZ-v14-PID.zip

The owner-private output includes:

- V14-READ-ME-FIRST.md: actual stage times and Ghidra function coverage.
- V14-STAGE-MANIFEST.json: complete success/failure data.
- V14-85-QUESTION-EVIDENCE-LEDGER.json: 85 questions, evidence sources,
  probe execution flags, explicit status and remaining proof boundaries.
- V14-PROPOSITION-EVIDENCE.json: concrete facts with evidence pointers.
- verified-prior-v13/*: SHA-checked relevant original V13 static evidence.
- v14-full-p1-ghidra.json: entire P1 recognized-function scan.
- v14-full-p3-ghidra.json: entire P3 recognized-function scan.
- updater-v11/*: SHA-validated static analysis from the updater track.
- offline-boot-assets-v13.json and offline-rootfs-and-boot-map-v12.json.
- v14-arm-qemu-boot-fixture.json and guest serial transcript.
- ZIP_INDEX.json: per-file SHA256 hashes and byte lengths.

The original owner eMMC backups and firmware bytes are NOT bundled.

## Strict interpretation of question statuses

- ANSWERED_STATIC: specifically evidenced static proposition, including
  source path and precise statement.
- PARTIAL_STATIC_NEEDS_FURTHER_PROOF: some subclaims proven, others not.
- ANSWERED_VM_SCOPE: actual tested generic ARM virt behavior, not physical RK3288.
- ONLY_SYNTHETIC_PREVIOUS_V13: old 42 synthetic Fastboot policy tests only.
- PROBED_UNRESOLVED: relevant scan happened, but not enough evidence to answer.
- NOT_PROBED_STAGE_FAILED: relevant run did not succeed; explicitly not counted.
- NEEDS_HARDWARE_CONFIRMATION: immutable ROM, fuses, wiring, bootloader
  physical behavior or independent restoration cannot be proven by static/QEMU.

**Completing the 85-question inventory is not the objective. Finding and
closing the maximum number of real, verifiable questions is.** Many questions
will remain hardware-only and should never be marked closed from a simulated
test or a long-running decompiler.

## Safety and caveats

Read-only owner inputs; the only files created are private reports, synthetic
QEMU fixture files, temporary imported Ghidra projects, and ZIP outputs
under ~/mpfree-work. Source-only Ghidra reports include proprietary derived
disassembly and MUST NOT be pushed to public GitHub. Ghidra uses
-readOnly -noanalysis on the owner-saved P1/P3 projects; imported updater
ELFs go only to disposable projects.

V14 does not attempt physical Fastboot unlock, flash, erase, bootloader RAM
image transmission, U-Boot saveenv, eMMC modification, destructive recovery,
OTP/eFuse writes, or actual MaskROM entry. The physical recovery questions
still require separately approved hardware work after non-destructive
recovery evidence.

Unlike V12/V13, V14 may take significant time depending on actual
decompiler and QEMU performance. There is no artificial delay; elapsed time
and number of functions genuinely decompiled are measured and disclosed.
