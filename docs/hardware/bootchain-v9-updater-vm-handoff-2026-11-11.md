# MPFree V8 owner report → V9 RockUSB/updater and QEMU kernel handoff

Date: 2026-11-11 (Pacific). This document describes static owner evidence;
the private Ghidra reports MUST NOT be committed or redistributed.

## Verified inputs

Owner V8 ZIP: bootchain-wide-20261112T004329Z-v8-3641449.zip.
ZIP CRC passes, 14 SHA256-indexed entries verified, plus ZIP_INDEX.json.
All four Ghidra post-scripts completed and emitted schema-validated JSON:

| Role | Verified SHA prefix | Recognized funcs | Selected | Pseudocode |
| --- | --- | ---: | ---: | ---: |
| az01-update ELF | 57511e1d | 60 | 60 | 60 |
| libaz01-prog.so.1.3 | af613a88 | 423 | 340 | 340 |
| P1/SRAM SPL | 6faade10 | 213 | 180 | 180 |
| P3/U-Boot | b4d3940c | 1050 | 260 | 260 |

WARNING: Pseudocode counts include tiny PLT/import stubs and
bad-instruction placeholders, not just fully recovered real functions.
Stripped updater main is not yet resolved. Actual Ghidra report
status success does NOT mean complete analysis.

## Findings from V8

- az01_open_image (lib ELF virtual 0x1738c) recognizes AZ01, AZ0x and
  a third blob beginning D0 0D FE ED (FDT magic). Dispatch occurs
  through format-specific tables; trust requirements may differ.
- az01_write_image (0x1e194) calls az01_verify_image(image, 2),
  compatibility checks, az01_prepare_to_write, and partition
  iteration/writing. This is NOT proof that every write path requires
  a verified signature, because verification state flags and
  device-backend dispatch are not yet proven.
- az01_verify_image (0x17554) caches verification status in an
  image object and has a parameter-dependent early-return path.
  The semantics of previous status 2 and flag parameter 2 remain
  high-priority questions.
- az01_write_partition_image (0x1798c) dispatches image type 1
  to az01_write_bootloader; other types use partition writing/
  streaming. It handles xz/LZMA, including a bounded bootloader
  decompression branch. Exact target partitions not yet proven.
- az01_write_bootloader (0x14b98) and az01_write_partition (0x14c50)
  dispatch to backend function pointers held at context offsets
  +0x18 and +0x1c. az01_stream_partition uses +0x20. Identify
  every table assigned to context+0x18 before asserting how
  RockUSB/fastboot/eMMC writes are performed.
- az01_verify_signature (0x1f094) invokes OpenSSL
  EVP_DigestVerifyInit with SHA256 and EVP_DigestVerify over
  signature length 0x100 bytes. This proves a verification
  implementation exists, not that all flows enforce it.
- az01_download_image (0x1b9bc) uses libcurl, then an
  onboard-key image verification before invoking az01_apply_image;
  this is one identifiable signed-download path.
- libaz01-prog contains meaningful ROCKUSB breadcrumbs:
  rockusb.c, rockusb_write_block, /dev/bus/usb/%03u/%03u,
  /sys/bus/usb/devices, uboot-spl, write-boot, oem
  flash-bootloader, oem update-bootloader, getvar, and
  reboot-bootloader. ROM/loader/fastboot protocol selection
  is NOT yet resolved. These strings alone never authorize
  USB writes to the actual MPC.
- libaz01-prog also references /usr/sbin/kexec, /boot/zImage,
  --kexec-syscall-auto and reboot --reboot-argument=update /
  loader / factory-reset. Isolate the exact state machine:
  no assumption that kexec is the only update mechanism.
- Updater RAMFS: 3,747,840 bytes, SHA256
  ca9299c050173ebe0912d001c1a137c0e6dda8001987c2b423cbf8b4158fc7af.
  The complete unsquashfs path list contains **272 paths**, mostly
  BusyBox multi-call links and armhf libraries. It has few
  update-named paths (boot, usr/sbin/reboot) and only three
  inspected small config files. Do NOT infer a complete
  updater implementation exists inside this minimal image.
  Inspect file TYPES, symlinks, process entrypoints and
  how zImage/ramfs are launched before inferring roles.
- P1/P3 latest focused decompilation recovers the known
  media-selection, handoff, bootmode and Fastboot paths.
  It does not demonstrate physical ROM MaskROM entry, USB
  wiring, RAM safety or independent eMMC restore.

## Exact V9 investigation: what to answer next

V9 adds a private SHA-checked Ghidra pass directed at these gaps:

- Recover stripped az01-update main and callbacks using ARM32
  R_ARM_RELATIVE relocation slots into executable .text,
  where the importer recognizes only PLT stubs.
- Locate every device-backend vtable implementing bootloader
  writer (+0x18), partition writer (+0x1c), and stream writer
  (+0x20), and trace backend selection (onboard eMMC,
  Fastboot, RockUSB).
- Reconstruct rockusb USB discovery, interface, command,
  read/write protocol and error handling; distinguish ROM
  from loader-mode dependencies.
- Trace image verification flag 2/state 2 and whether every
  real write path is dominated by authentication.
- Map images type=1/2/0, PART sections, compatibility,
  decompression and destination.
- Distinguish updater kexec from recovery reboot and
  whether separate ramfs is an init environment or merely
  an external support root.
- Re-check SPL and U-Boot callback neighborhoods for direct
  SD boot, boot-ROM return versus USB MaskROM, Fastboot
  authorization and failure reset.

Run from the updated owner worktree:

    git -C "$HOME/mpfree-work/mpfree-wide-audit-src" fetch origin research/wide-bootchain-audit-20261111
    git -C "$HOME/mpfree-work/mpfree-wide-audit-src" switch --detach FETCH_HEAD
    python3 "$HOME/mpfree-work/mpfree-wide-audit-src/tools/mpc-one/run_boot_updater_v9.py" \
      --prior-zip "$HOME/mpfree-work/bootchain-wide-20261112T004329Z-v8-3641449.zip"

Output: ONE private bootchain-wide-*-v9-<pid>.zip, with
v9-ghidra-az01-update.json, v9-ghidra-libaz01-prog.json,
v9-ghidra-spl.json, v9-ghidra-uboot.json, Ghidra logs,
verified archive lineage, ARM ELF relocation candidates,
RockUSB/update evidence questions and RAMFS file-type inventory.
If a Ghidra postscript returns exit 0 but fails to produce the
expected schema/SHA report, stage success stays false.

## VM kernel compile failure and fix

Owner VM preparations completed:
- separate cloned rootfs + COW overlay produced successfully;
  clone SHA256 674fc1978a9f588dec556aa002953e4f0c5abadd467e07de2adf32a0466ecabb.
- kernel build uses installed source
  /usr/src/linux-source-7.0.0/linux-source-7.0.0.tar.bz2.
- config verified the needed virtio, PL011, ext4 builtins.
- FIRST compile failure in TI Keystone-only
  drivers/soc/ti/knav_qmss_queue.c:1860:
  undeclared identifier knav_qdev in knav_queue_probe.
  This is unrelated to QEMU ARM "virt" and independently
  reported in Linux stable reviews in September 2026.
  The corrected guest kernel builder disables
  KEYSTONE_NAVIGATOR_QMSS, KEYSTONE_NAVIGATOR_DMA,
  ARCH_KEYSTONE and SOC_TI; re-runs olddefconfig and
  asserts the unwanted driver is absent. Source is untouched.

For the owner machine:

    git -C "$HOME/mpfree-work/mpfree-wide-audit-src" fetch origin research/wide-bootchain-audit-20261111
    git -C "$HOME/mpfree-work/mpfree-wide-audit-src" switch --detach FETCH_HEAD
    set -o pipefail
    bash "$HOME/mpfree-work/mpfree-wide-audit-src/tools/mpc-one/build_qemu_arm_kernel.sh" \
      2>&1 | tee "$HOME/mpfree-work/mpfree-arm-qemu-lab/kernel-build-retry.log"

No second --prepare: both mpc-310-lab-base.ext4 and
mpc-310-lab.qcow2 already exist. If zImage builds successfully,
explicitly launch with --launch and capture shell output.
Do not execute Akai updater, program-fuses, or write hardware.
QEMU ARM virt does NOT emulate RK3288 BootROM/DDR/USB recovery.
For unknown firmware execution, run the ARM emulator inside an
isolated outer x86 KVM guest, without writable host shares or
physical USB/eMMC passthrough. An offline static report and
successful VM rollback do not prove a real-MPC recovery path.

## Linux upstream references

Kernel QMSS configuration:
https://www.cateee.net/lkddb/web-lkddb/KEYSTONE_NAVIGATOR_QMSS.html
Issue reported independently in 2026:
https://lkml.iu.edu/hypermail/linux/kernel/2609.1/14429.html
QEMU ARM virt board and security limits:
https://www.qemu.org/docs/master/system/arm/virt
https://www.qemu.org/docs/master/system/security
