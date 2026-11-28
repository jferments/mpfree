# MPFree V10 owner results, V11 backend hypothesis and VM next steps

Date 2026-11-11 Pacific. Owner firmware, proprietary Ghidra output, and private ZIPs remain local. This public document reports only addresses and high-level reverse-engineered behavior.

## V10 evidence integrity and VM status

Owner ZIP: bootchain-wide-20261112T014736Z-v10-3708829.zip.
15 SHA-indexed files verified against their manifest. ZIP CRC passes.
Prior V9 lineage was verified.
Ghidra reports: az01-update 7 functions; libaz01-prog 112;
P1 SPL 13; P3 U-Boot 56. Every V10 stage succeeded.
Ghidra selected-function counts do NOT measure total firmware coverage.

ARM QEMU virt Cortex-A15 booted Linux 7.0.14 with the cloned firmware 3.10
ext4 rootfs (virtio /dev/vda, root=/dev/vda init=/bin/sh). Both CPUs
and /proc, /sys worked. The updater, libaz01-prog and updater ramfs
were present at expected locations. User stopped QEMU with SIGINT,
not a kernel panic. The ephemeral -snapshot mode discards guest writes.
The PCI probe warning -75 is unrelated to successful rootfs boot.

## V10 major new discovery: backend callback-table base alignment

The V10 ELF relocation report, interpreted in ELF virtual address space,
contains a contiguous cluster around 0x12a74. The stronger candidate
backend table is at **0x12a74**, NOT the overlapping zero-header
candidate at 0x12a80. Evidence:

| Offset | Function pointer (ELF vaddr) | Static classification |
| ------ | ---------------------------- | --------------------- |
| +0x00 | value 2 | small header/type marker |
| +0x04 | value 0 | small header/mode |
| +0x08 | 0x54b8 | az01_device_get_usb_ids and transport setup |
| +0x18 | 0x5bec | bootloader transfer with receive/ACK checking |
| +0x1c | 0x6c14 | partition/block writer |
| +0x20 | 0x6dbc | streaming/block writer |
| +0x2c | 0x6028 | device command/commit candidate |
| +0x34 | 0x5da8 | backend control/reset candidate |

The generic dispatch wrappers az01_open_device, az01_write_bootloader,
az01_write_partition, az01_stream_partition, az01_commit_bootloader
and az01_do_reset dereference callbacks at offsets
+0x08, +0x18, +0x1c, +0x20, +0x2c, +0x34 respectively.
This offset alignment AND matching function behavior strongly suggests
a device programming callback table.

A second plausible table is **0x12bd8**, with marker 1, header word 0,
and pointers at +0x08 (0xc73c), +0x18 (0xc49c),
+0x1c (0xc3fc), +0x34 (0xc8c8).
The 0xc49c routine recognizes FDT-containing loaders; its children
use RockUSB-style commands. The 0xc8c8 routine builds a 31-byte USB
USBC command block and consumes 13-byte response. This suggests
another backend mode, but exact device selector remains unknown.

CAUTION: Other overlapping pointer arrays at 0x12a80, 0x12ae0,
0x12b10, 0x12b48 etc. also superficially satisfy the same
offset template; some represent image parser/format callbacks
rather than storage device interfaces. The candidate report is
not proof of backend identity. No semantic claim is justified
until the table address is actually assigned to the correct
device context and the calling convention is validated.

### V10 analysis bug and repair

V10's report populated every possible_relative_function as null,
even when the memory word was the address of a recognized callback.
Cause: Ghidra had already applied the PIE image base to some
ELF relocation pointers (example word 0x15bec); V10 then added
image base 0x10000 again and searched for the wrong address.
The raw V10 word evidence is still useful. V11 interprets raw ELF
vaddrs independently from imported Ghidra addresses, and in
Ghidra checks the *already-relocated* word first.

## Signature and recovery gates

az01_verify_image at 0x17554 has state==1 success and state!=0
with flag 2 success. A format verifier returning -13 stores state 2.
az01_write_image calls verify_image(image,2) before write preparation.
The meaning of -13, whether the state can be reached in each format,
and whether a dangerous write can be reached under that state
remain unproved. Avoid claiming a universal bypass.

U-Boot's Fastboot flash and erase handlers still check
mpf_fastboot_permission; the initial permission value and
runtime conditions enabling OEM commands are not established.
Presence of RockUSB, U-Boot, or Fastboot code does not demonstrate
that the physical MPC supports independent RK3288 MaskROM USB
access or recovery without factory bootloader.

## V11 deliverables

- tools/mpc-one/offline-bootchain-v11-backend-tables.py:
  scan verified ARM ELF R_ARM_RELATIVE entries, reconstruct candidate
  callback tables and their compact headers, preserve ambiguity.
- tools/ghidra/MPFreeBackendSelectorV11.py:
  fix PIE image-base pointer interpretation, decompile callback
  functions and one-hop callers/callees, inspect data xrefs, continue
  P1 SPL and P3 U-Boot boot/permission neighborhoods with SHA checks.
- tools/mpc-one/run_boot_updater_v11.py:
  verify owner 3.10 rootfs and backup, verify the V10 ZIP index and
  four source hashes, write one private ZIP with audit stages and
  logs. It imports updater ELFs into disposable Ghidra projects;
  opens saved SPL/U-Boot projects -readOnly -noanalysis.
- tools/mpc-one/tests/test_boot_updater_v11.py:
  synthetic regression on table alignment and source checks.

### Owner commands

    git -C "$HOME/mpfree-work/mpfree-wide-audit-src" fetch origin research/wide-bootchain-audit-20261111
    git -C "$HOME/mpfree-work/mpfree-wide-audit-src" switch --detach FETCH_HEAD
    python3 "$HOME/mpfree-work/mpfree-wide-audit-src/tools/mpc-one/run_boot_updater_v11.py" --prior-v10 "$HOME/mpfree-work/bootchain-wide-20261112T014736Z-v10-3708829.zip"

Upload only generated bootchain-wide-*-v11-*.zip. No need to rerun
prior broad audits or rebuild QEMU kernel.

## VM next stage

The generic 3.10 userspace boot is demonstrated. Repeating uname,
cat /proc/cpuinfo, and ls now adds little evidence.

For genuinely new runtime information, the next controlled experiment
should exercise MPFree's own synthetic image parsers and a mock USB/eMMC
backend in the QEMU ARM guest, with no device passthrough, no network,
and ephemeral disk snapshots. Record serialized mock commands,
error codes, retries, and state transitions; compare them to our
static updater function graphs.

When considering execution of vendor updater binaries for
differential behavior, first run QEMU inside an additional isolated
x86 KVM guest without writable shares or physical USB/eMMC.
Connect vendor executables only to a fake USB transport and fake
file/block devices that reject or log write requests. Do not run
factory updater, program-fuses, or flash commands on the actual MPC.

Physical bootloader changes remain blocked until independent silicon
ROM/MaskROM recovery is demonstrated and a full hardware restore
procedure works without Akai Linux or factory bootloader. Emulator
rollback is not a substitute for physical recovery.
