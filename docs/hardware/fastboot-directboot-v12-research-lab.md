# MPFree V12 Fastboot and Direct-Boot Research Lab

**Purpose:** one read-only, concurrent Ghidra plus isolated ARM QEMU research round.
**Branch:** research/fastboot-directboot-lab-20261112
**Safety:** no MPC SSH, no device USB, no eMMC commands, no ROM transmission,
no vendor updater execution, no firmware modification.

## One command (ML-tower)

Use the existing separate broad-audit worktree, not your main development tree.

~~~bash
R="$HOME/mpfree-work/mpfree-wide-audit-src" && \
git -C "$R" fetch origin research/fastboot-directboot-lab-20261112 && \
git -C "$R" switch --detach FETCH_HEAD && \
python3 -m py_compile \
 "$R/tools/mpc-one/run_fastboot_directboot_v12.py" \
 "$R/tools/mpc-one/tests/test_fastboot_directboot_v12.py" && \
python3 -m unittest discover \
 -s "$R/tools/mpc-one/tests" \
 -p 'test_fastboot_directboot_v12.py' -v && \
python3 "$R/tools/mpc-one/run_fastboot_directboot_v12.py"
~~~

Produces one private ZIP at
~/mpfree-work/bootchain-wide-UTC-v12-PID.zip
plus a checksum sidecar (.zip.sha256). Upload only the ZIP, never proprietary
Ghidra reports or source firmware to a public GitHub repository.

## Concurrent jobs

1. Ghidra P1 SPL: exact pinned source SHA256; ROM/SPL startup, boot media,
   loaded image handoff and recovery, all READ ONLY on saved analyzed projects.
2. Ghidra P3 U-Boot: exact pinned source SHA256; OEM Fastboot unlock string
   and references, authorization global, flash/erase/download/boot dispatch,
   image formats, bootm/bootz, MMC/SD, recovery button and PMU bootcounter.
3. Isolated ARM QEMU virt: reuses the owner's existing cloned 3.10 rootfs
   and overlay, -snapshot, no networking or USB passthrough. Builds an
   independently authored ARM test model, places it in a synthetic read-only
   ext4 fixture, and executes the model in the ARM guest.
4. Native C model: independent local compilation of the same synthetic
   test cases for cross-check.
5. Offline regular-file rootfs inventory: verifies the saved P1 eMMC hash,
   rootfs ext4 signature, and inspects /boot using debugfs read-only.

Ghidra P1, Ghidra P3, QEMU, native test, and offline inventory run concurrently.
Partial failures are recorded, and remaining jobs proceed.

## Prerequisites

The prior broad-audit workspace contains the pinned saved Ghidra projects
in ~/mpfree-work/ghidra/projects. Ghidra is installed through Snap or
GHIDRA_HOME. The ARM QEMU clone/overlay/kernel were prepared in the earlier
integrated lab. Host tools include Python 3.10+, arm-linux-gnueabihf-gcc,
qemu-system-arm, qemu-img, mkfs.ext4 and cc. debugfs is optional.

By default the runner uses the verified existing backup and extracted 3.10
rootfs paths from the previous research thread. Optional arguments:

~~~bash
python3 tools/mpc-one/run_fastboot_directboot_v12.py \
  --rootfs /absolute/path/to/extracted-rootfs.img \
  --backup /absolute/path/to/verified-emmc.img
~~~

For a host-only experiment, with no QEMU boot:

~~~bash
python3 tools/mpc-one/run_fastboot_directboot_v12.py --skip-qemu
~~~

## ZIP contents

- V12-READ-ME-FIRST.md: concise stage outcomes and hypotheses.
- V12-STAGE-MANIFEST.json: exact status for each investigation.
- V12-RESEARCH-QUESTION-MATRIX.json: 70+ enumerated questions.
- ghidra-p3-fastboot-v12.json: private authorization/direct-boot evidence.
- ghidra-p1-fastboot-v12.json: private early-boot evidence.
- offline-rootfs-and-boot-map-v12.json: verified source-file metadata.
- native-synthetic-results.log: C model behavior on host.
- qemu-v12-console.log and qemu-v12-stage.json: VM tests.
- ZIP_INDEX.json: file sizes and SHA256 for each included report.

Raw proprietary firmware, user original eMMC images, any original executable,
and compiled synthetic test binaries are NOT bundled.

## Interpretations and limitations

The C model deliberately uses a fictitious owner authorization token.
Its assertions are proposed engineering safety invariants, NOT a replay of
Akai Fastboot, not a demonstrated vendor unlock, and not a live RK3288
execution environment.

The guest is QEMU ARM virt, NOT an RK3288 board emulator. No actual SPL,
U-Boot, RockUSB, Fastboot or vendor updater is executed in it. If Ghidra
finds an OEM unlock string, that proves only static presence; trace the
actual dispatcher and state changes before claiming live behavior.

True MaskROM entry, vendor Fastboot RAM boot, real eMMC restoration, user
rootfs flashing and physical boot selection remain UNPROVEN by this suite.

No physical flash/erase, OTP or recovery modifications are permitted.
Follow-up should classify each question as answered by static evidence,
answered by synthetic model only, needs isolated vendor-code execution,
or requires separately authorized hardware proof.
