# MPFree V8 — Deep Akai updater and RK3288 bootchain static analysis + ARM VM lab

Date: 2026-11-11. This is a **read-only, owner-controlled** analysis plan. All generated Ghidra pseudocode is PRIVATE and must never be committed to the public repository.

## Evidence already established

- Owner raw eMMC user-area backup includes SHA-verified SRAM SPL and vendor U-Boot executable, both previously imported to private Ghidra projects. No physical MaskROM recovery has been demonstrated.
- Official Akai 3.10 Gen1 update container reportedly contains a rootfs payload, not an explicit SPL/U-Boot partition payload. That does **not** prove the installed userspace updater is unable to write bootloaders: separate updater code paths must be examined.
- The 3.10 ext4 rootfs includes `/usr/sbin/az01-update`, `/usr/lib/libaz01-prog.so.1.3`, and `/opt/ramfs-update.squashfs`; the latter was previously listed but its contents were not exhaustively explored.
- `libaz01-prog.so.1.3` exposes/contains update-related symbols including bootloader writing/commit and signature verification. Invocation conditions remain unknown.
- `az01config` contains `program-fuses`, and firmware 3.10 includes a bootloader digest check and Linux LoadPin service. NEVER invoke unknown vendor maintenance commands on the real MPC to discover their behavior.
- Factory boot scripts appear equivalent between 3.6 and 3.10. This is not proof of verified independent SD selection or unsigned SPL acceptance.

## The new offline V8 Ghidra pass

Files:

- `tools/ghidra/MPFreeDirectedBootUpdaterV8.py`: decompiles prioritized functions and their callers/callees, selected strings/xrefs, all recognized indirect callsites. Runs against preserved P1/P3 projects with `-readOnly -noanalysis`, and the updater ELF/library in ephemeral Ghidra import projects.
- `tools/mpc-one/run_boot_updater_v8.py`: orchestrates work on the owner's offline SHA-validated eMMC backup and extracted 3.10 ext4 rootfs; uses debugfs to recover the two named updater files and `/opt/ramfs-update.squashfs`; uses `unsquashfs -l/-cat` to read selected updater RAMFS contents into temporary working files, never executing them. Optional ELF files in updater RAMFS are imported in private disposable Ghidra projects. One private SHA-indexed ZIP at exit, even when some optional analyses fail.

Use `bash` / `python3` in the existing detached research worktree, not the MPC.

```bash
git -C "$HOME/mpfree-work/mpfree-wide-audit-src" fetch origin research/wide-bootchain-audit-20261111 && \
git -C "$HOME/mpfree-work/mpfree-wide-audit-src" switch --detach FETCH_HEAD && \
python3 "$HOME/mpfree-work/mpfree-wide-audit-src/tools/mpc-one/run_boot_updater_v8.py"
```

The runner writes a **single** private `~/mpfree-work/bootchain-wide-*-v8-<pid>.zip`. Upload only that ZIP; do not commit its proprietary pseudocode or binaries.

## Prioritized updater questions to answer (U01–U22)

| ID | Question | Evidence required |
| --- | --- | --- |
| U01 | What calls `az01-update` and under which systemd/reboot state? | Service/script/binary xrefs and startup graph |
| U02 | Which code paths parse the official `AZ01` update container? | Ghidra caller tree, header/length/CRC checks |
| U03 | Where does signature verification occur? | `az01_verify_signature` and all concrete callers with error branches |
| U04 | Does image authentication precede EVERY partition-write code path? | Complete authorization gate dominance in control flow; static xrefs aren't enough |
| U05 | How does `az01-update` choose the `rootfs`, `bootloader` or recovery target? | Call arguments and signed container PART record mapping |
| U06 | What do `az01_write_bootloader` / `az01_commit_bootloader` actually write? | Device path, partition numbers, offsets, lengths, fsync, error handling |
| U07 | Does the updater write eMMC boot0/boot1 or only user LBAs? | Exact `/dev/mmcblk*` paths and kernel driver APIs |
| U08 | How does it protect against power interruption? | Dual-slot/fallback update transaction order, journal, checksum, reboot sequence |
| U09 | Does the updater rely on a primary/fallback bootloader digest equality check? | Call path from update entry to hash checker, not just binary string hits |
| U10 | Is downgrade/version rollback enforced by userspace? | Compare version predicates and persistent metadata |
| U11 | Can the official rootfs-only 3.10 package invoke bootloader-writing code? | Concrete PART selection and execution condition |
| U12 | What is inside `ramfs-update.squashfs`? | Verified file list, init/update scripts, native helpers, exec order |
| U13 | Does the updater ramfs mount/write the eMMC outside normal Linux rootfs? | Script/call graph, mount targets, IPC and reboot modes |
| U14 | Does the updater configure eMMC `PARTITION_CONFIG`, boot0/boot1, or EXT_CSD? | ioctl/MMC command evidence, independent physical runtime confirmation |
| U15 | What does the `program-fuses` operation call? | Static dispatch and call graph only; never execute directly |
| U16 | Are fuse-related operations truly irreversible silicon OTP or ordinary settings? | Source-level target evidence and RK3288 docs; no write experiments |
| U17 | Can an owner installer safely reuse the *image-format parser*? | Extract functional specification and test parser against synthetic images |
| U18 | Can we avoid original proprietary updater entirely? | Build own open-source verified partition writer, recovery/rollback design |
| U19 | Does factory U-Boot load any update ramdisk directly? | P2 boot environment, fastboot/recovery code and boot scripts |
| U20 | Can SD-first selection survive future kexec removal? | Physical U-Boot state + nonpersistent boot selection |
| U21 | What are true MaskROM/USB ROM dependencies? | SPL return-to-ROM vs actual ROM enumeration, no static proof conflation |
| U22 | What immutable recovery evidence must precede bootloader changes? | Boot0/boot1/user coverage; independent external loader, restore rehearsal |

## VM: after successful ext4+COW preflight

Owner preflight output verified the extracted 3.10 raw ext4 filesystem (451,468,288 bytes) and created `~/mpfree-work/mpfree-arm-qemu-lab/mpc-310.qcow2`. This original overlay should be left alone. The new VM helper makes an additional **separately cloned base image** and separate `mpc-310-lab.qcow2` to avoid referencing the owner's source image.

### VM0 — install packages (actual owner APT sources: Ubuntu 26.04 resolute)

The owner's APT output explicitly uses the `resolute` suite.
`qemu-user-static` is now a virtual package; use `qemu-user`.
The `linux-source` metapackage installs the available 7.0 source.

```bash
sudo apt update && sudo apt install -y \
  qemu-system-arm qemu-user qemu-utils squashfs-tools \
  gcc-arm-linux-gnueabihf binutils-arm-linux-gnueabihf \
  make bc flex bison libssl-dev libelf-dev linux-source
```

Verify with `command -v qemu-system-arm`, `command -v qemu-arm`,
and `ls -lh /usr/src/linux-source-*/linux-source-*.tar.*`.
The kernel builder discovers installed version 6.8/7.0 rather than
hard-coding a version. This does not change the host's running kernel.

Apt installs host tools and kernel-source packages; no MPC is accessed.

### VM1 — make a separate clone and disposable overlay

```bash
python3 "$HOME/mpfree-work/mpfree-wide-audit-src/tools/mpc-one/arm_qemu_lab.py" --prepare
```

This copies and SHA256-verifies the already extracted ext4 file to `mpc-310-lab-base.ext4`, chmods the cloned base read-only, then creates `mpc-310-lab.qcow2` backed by the **clone**, not the owner's original rootfs. It refuses to overwrite existing files. It does not reuse or overwrite `mpc-310.qcow2` from the earlier preflight.

### VM2 — build kernel for QEMU `virt` instead of Rockchip RK3288

```bash
bash "$HOME/mpfree-work/mpfree-wide-audit-src/tools/mpc-one/build_qemu_arm_kernel.sh"
```

This unpacks the locally installed Ubuntu kernel source in `~/mpfree-work/mpfree-arm-qemu-lab/linux-arm-src`, configures `multi_v7_defconfig` plus built-in ARM virt, PL011, virtio-mmio/block, ext4, devtmpfs, procfs, and compiles `linux-arm-build/arch/arm/boot/zImage` with the ARMhf cross compiler. The stock Akai kernel/DTB and SPL/U-Boot binary will **not** boot unmodified under `-M virt`.

### VM3 — boot to a minimal shell, NO automatic Akai startup services

```bash
python3 "$HOME/mpfree-work/mpfree-wide-audit-src/tools/mpc-one/arm_qemu_lab.py" --launch
```

The `--launch` action is an explicit request to start a local QEMU ARM32 process with 2 GiB guest RAM, a `cortex-a15` model, `console=ttyAMA0`, virtio-block qcow2 storage and `init=/bin/sh`. Network, QEMU monitor, physical-device passthrough and writable host shares are disabled. No original MPC code is executed as an early bootloader.

In the guest, once the shell appears, collect:

```sh
uname -a
cat /proc/cmdline
cat /proc/cpuinfo
ls -la /usr/sbin/az01-update /usr/lib/libaz01-prog.so.1.3
```

If the shell starts without procfs, `mount -t proc proc /proc` and `mount -t sysfs sysfs /sys` can be performed *inside the disposable guest* if its user tools support them. Boot to shell first; **do not** start systemd or run updater utilities in this initial step. Exit QEMU with Ctrl-C. Collect the console output for the next MPFree research iteration.

If QEMU immediately panics after `init=/bin/sh`, investigate its `/bin/sh` symlink and dynamic linker in the 3.10 filesystem using read-only `debugfs`; this can be a missing ARM ABI/shared-library dependency rather than a kernel boot failure.

### VM4 — trace-safe instrumented updater tests (later, separately gated)

Start by building a *mock* updater model from verified Ghidra behavior, synthetic `AZ01` records, and disposable virtual eMMC partition files. Only after an **outer x86-64 KVM VM** with no physical device passthrough and no writable host shares is established should unknown vendor maintenance binaries be considered for execution. QEMU ARM-on-x86 uses TCG, which QEMU's own documentation does not provide as a hardened security boundary; an outer KVM guest supplies another isolation layer.

Runtime evidence from the ARM guest (syscalls, opened paths, return codes, error messages, timing, process fork/exec, and failure cases) should be used to **rank Ghidra callsites** and annotate actual versus unreachable branches. Specifically, when a virtual test reports `openat("/dev/mmcblk0pX")`, we can use Ghidra xrefs to find the exact path and parent function; if the test calls `ioctl(fd,... )`, trace into the surrounding device-interface wrapper; if it rejects a corrupt input, identify the predicate and error branch before proposing owner installer logic. Follow with a second static pass on those functions and compare 3.6 against 3.10.

### Physical recovery remains a separate program

A virtual disk reset does **not** prove recovery from a damaged RK3288 SPL, U-Boot, eMMC boot partition, RPMB or OTP setting. Physical recovery requires validated silicon ROM entry, independent loader execution, independent boot-region readback/restore and sacrificial restoration rehearsal. **No irreversible `program-fuses`, bootpart settings, raw mmc writes, flash/erase or update execution is authorized by these scripts.**

## 2026-11-11 V8 owner audit failure and corrected rerun

Owner V8 ZIP: bootchain-wide-20261111T235450Z-v8-3624301.zip.
All 10 indexed log/metadata files passed ZIP CRC, sizes and SHA256.
The owner Ghidra Snap is Ghidra 12.2 / embedded Python 3.14.
Both updater ELF imports succeeded and the SPL/U-Boot projects
opened successfully, but **all four V8 post-scripts failed**:
PyGhidra's Java-backed script namespace proxy raised AttributeError
(property fget has no setter) while assigning a module-level
loop variable at line 116. Ghidra returned exit code 0 despite
the post-script exception. The runner correctly marked all four
stages success=false. No v8-ghidra-*.json report was generated.

Fix: all mutable V8 Ghidra script state is now inside a regular
Python function mpfree_v8_entrypoint(), preventing module-level
loop targets from being assigned through a Java attribute proxy.
A regression check asserts no loops remain at module scope.

The RAMFS is 3,747,840 bytes, SHA256
ca9299c050173ebe0912d001c1a137c0e6dda8001987c2b423cbf8b4158fc7af.
Initial inspection reported only 2 update-named paths:
boot, usr/sbin/reboot; no selected contents. These were NOT
the complete SquashFS contents. The corrected runner records
up to 2400 paths and examines /init, linuxrc, rcS and other
startup/config candidates regardless of filename.

Rerun:

    python3 "$HOME/mpfree-work/mpfree-wide-audit-src/tools/mpc-one/run_boot_updater_v8.py"

Expect one private bootchain-wide-*-v8-<pid>.zip. Four nonempty
Ghidra JSON reports should appear if the corrected scripts work;
if a script fails, preserve the log and stage manifest.

The owner APT sources use Ubuntu **26.04 resolute**, not 24.04.
Use qemu-user (qemu-user-static is a virtual package) and
linux-source (currently linux 7.0). The kernel builder now
discovers the installed source tarball in /usr/src rather
than requiring Linux 6.8. Do not install older Ubuntu archives
merely to satisfy an obsolete package name.

No live MPC commands, ROM access, firmware writes or updater
execution are authorized by this analysis. QEMU virt does not
simulate true RK3288 silicon recovery.
