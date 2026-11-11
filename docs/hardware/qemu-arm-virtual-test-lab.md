# MPFree: V7 findings, safe runtime questions, and ARM QEMU lab architecture

Updated 2026-11-11. Owner private source archive:
bootchain-wide-20261111T231303Z-v7-3609390.zip.
No live-device experiment has been performed by this analysis.

## Verified V7 archive results

- ZIP CRC verification passed. Its 61 indexed report/log files all matched their
  ZIP_INDEX recorded sizes and SHA256 hashes; plus one ZIP_INDEX.json entry.
- V7 stages elf-dispatch, ARM unwind, bounded binary-reference search and V7
  synthesis all exited 0.
- az01config: two overlapping clusters of actual relocation-backed command
  table data, rather than six independent interfaces. Table at 0x3df0:
  help->0x0cc8; show->0x1aac; set->0x1874; clear-flag->0x1470;
  set-flag->0x1454. Property table at 0x3e34: flags, compatible,
  serial, panel-compatible, ethaddr. Handlers are candidate ELF-relative
  virtual addresses (no dynamic code execution performed).
- Program-fuses was NOT mapped to a handler by the V7 table parser.
  Literal program-fuses occurs in both az01config and libaz01-prog.so.1.3.
  That is a high-priority safety investigation, NOT permission to invoke
  any command. It could be in a separate code path.
- ARM EHABI .ARM.exidx produced only one candidate entry per small ELF,
  yielding little additional coverage.
- Bounded 3.10 binary/file-reference scan examined 473 candidate files,
  with 36 files containing at least one search term. There was **no
  external exact-name reference** to az01-bootloader-sig among scanned
  files; that is not proof no program invokes it. The LoadPin unit
  references its companion binary. libaz01-prog contains program-fuses.
- V6 synthesis reproduced by V7: 213/213 recognized SPL functions have
  pseudocode and 1044/1050 recognized U-Boot functions have pseudocode.
  Computed call sites 19 P1, 167 P3 remain unresolved. No static report
  proves a genuine silicon BootROM recovery mode or safe RAM boot.

## Two distinct kinds of virtualization

### Tier A: ARM user-mode execution (good for exact Akai *userspace*)

QEMU user-mode (\`qemu-arm-static\`) emulates the ARM32 instruction stream
for an individual executable and translates its Linux syscalls. Use a
restricted Linux mount/user namespace or disposable container with a
read-only staged rootfs and no host device access. It runs with the
host's kernel rather than emulating an ARM Linux kernel.

Useful for:
- Checking an ARM ELF's help/error/argument parsing after first
  reconstructing side effects from disassembly.
- Tracing filesystem path operations and library dependencies.
- Building tests with fake \`/dev/mmcblk*\`, mock ioctl handlers and
  failure injection against a throwaway filesystem.

Limitations: it is not an entire booted MPC One; vendor device ioctl
calls are not faithfully emulated; a process can issue real host
syscalls unless the sandbox is correctly configured. **Never execute
az01config, program-fuses, or any unknown firmware-writing utility on
the host directly**, even via qemu-arm-static. First establish a
separate unprivileged sandbox and verified no-device policy.

### Tier B: Complete ARM32 guest on QEMU -M virt (recommended lab)

QEMU's \`virt\` machine is intentionally generic. It can emulate an
ARMv7 Cortex-A15-compatible platform, a PL011 serial port, generic
interrupt controller, virtio storage and other generic peripherals.
It does *not* model Akai MPC One or Rockchip RK3288 devices. The real
RK3288 has Cortex-A17 cores and board-specific clock, DDR, PMU, MMC,
USB, GPIO, ADC, eFuse and boot-ROM logic.

A **generic ARM32 Linux kernel built for QEMU virt** with built-in
PL011 console, virtio-mmio/virtio-blk, ext4, devtmpfs, and generic
device-tree support can boot the *userspace* extracted from the
stock 3.6 P6 or 3.10 rootfs into a disposable guest. Give QEMU the
auto-generated \`virt\` DTB. The stock Rockchip zImage and RK3288
DTB should NOT be expected to boot unmodified on \`virt\`.

Useful for:
- Testing whether real Akai userspace binaries, systemd units and
  libraries work under an ARM32 kernel with sufficient API support.
- Differential testing of 3.6 vs 3.10 startup, update parsing,
  signature-check algorithms and LoadPin unit behavior.
- Crash, ENOENT, permission, timeout and storage-failure injection
  with fake guest-only devices and mock kernel interfaces.
- Building a new MPFree initramfs, service manager, boot scripts and
  updater tests independently of Akai's existing kernel and kexec.
- Running reproducible ARM32 userspace/ABI tests in CI.

Limitations: Akai hardware-bound services may fail; eMMC appears as
virtio /dev/vda (not /dev/mmcblk0); genuine eMMC boot0/boot1,
EXT_CSD boot settings, RPMB and eFuse are not present. Virtual
LoadPin behavior depends on a separate kernel built with that LSM.
A simulation outcome cannot prove *hardware* ROM recovery, physical
USB wiring, DDR training, SPL acceptance, vendor Fastboot state or
safe U-Boot RAM addresses.

### Tier C: Boot-flow logic tests with QEMU-compatible U-Boot

Build upstream U-Boot for QEMU's ARM \`virt\` board or run its sandbox
test framework. Recreate *behavioral equivalents* of the P2 boot
environment, boot.scr payload, MMC order and kernel/FDT loading from
synthetic disk images. Run fallback/rollback/failure tests and fuzz
boot script handling. Use **no proprietary bootloader in a public
artifact**.

This tests the reproducibility and ergonomics of MPFree's proposed
boot manager and boot scripts, but not the actual proprietary
U-Boot 2016 MMIO drivers, board-specific SPL or silicon BootROM.

### Tier D: Full RK3288 hardware emulation (not the next step)

Accurately booting the original ROM/SPL/P3 U-Boot image would require
an RK3288 machine model implementing the relevant clock and reset
controller, SRAM/DDR, MMC, GRF/PMU, USB OTG, UART, watchdog,
GPIO/ADC, efuses and ROM boot order, with correct side effects.
There is no grounds in current sources to claim stock QEMU's
generic \`virt\` machine supplies this. Even if developed, a custom
machine model remains an approximation of physical fuses, signals
and recovery behavior. Do not make a bootloader flash safety
decision based solely on it.

Sources: QEMU documentation:
https://www.qemu.org/docs/master/system/arm/virt ;
https://www.qemu.org/docs/master/system/target-arm ;
https://www.qemu.org/docs/master/user/main.html ;
https://www.qemu.org/docs/master/tools/qemu-img.html ;
https://www.qemu.org/docs/master/system/security.html

## Recommended ARM VM implementation

**Phase VM0 — evidence inventory**:
- Inspect the owner's 3.6 P6 and separately extracted 3.10 ext4
  superblocks, kernel ABIs, dynamic linker/ARM hard-float layout,
  systemd version, /etc/fstab, init selection, kernel modules,
  package-library dependencies and service hard dependencies.
- Discover QEMU machines with \`qemu-system-arm -machine help\`.
- Validate host free space, user/group permissions and exact source SHA.
- Do not use the 7+ GiB whole eMMC image as the VM's direct boot disk
  initially: its SPL/P3 offsets will not be interpreted by the QEMU
  virt machine or the virtual firmware.

**Phase VM1 — disposable rootfs overlay**:
- Work under ~/mpfree-work/mpfree-arm-qemu-lab (outside source repository).
- Extract or reuse the already owner-extracted 3.10 raw ext4
  \`rootfs.img\`; for 3.6 P6 use an offline exact-offset
  copy, never \`mount -o rw\` on owner backup.
- Validate the source is a regular ext4 *file*, not a block device.
- Create a qcow2 overlay with absolute read-only raw backing:
  \`qemu-img create -f qcow2 -F raw -b /private/rootfs.img guest.qcow2\`
- The backing image should be immutable to the QEMU account.
  For defense in depth use a dedicated unprivileged host user,
  no write permission to base images, no passed devices, no
  host file sharing, no direct bridge network, monitor disabled,
  and snapshot/recreate overlay for each test.
- If QEMU executes as the same user who can modify the backing
  image, a COW overlay alone does not constitute a 100% guarantee
  against host modifications or an operator accidentally using
  qemu-img commit. QEMU's machine-mode TCG on an x86 host is a
  useful testing facility, not necessarily a formal hardened
  guest-isolation guarantee.

The repository provides \`tools/mpc-one/qemu_arm_lab_preflight.py\`.
By default it only inspects metadata. \`--create-overlay\`
explicitly creates a new COW file and refuses overwrites. With a
verified generic \`virt\` kernel, \`--kernel\` prints a proposed
QEMU command but *never executes it*.

**Phase VM2 — controlled first boot**:
- Use a known QEMU virt-compatible ARM32 zImage with console=ttyAMA0,
  root=/dev/vda, ext4 and virtio block built in, and QEMU-generated
  DTB (not the RK3288 DTB).
- First boot with \`init=/bin/sh\` or a deliberately small test init.
  Do not launch unknown Akai hardware management services on
  first boot.
- Once /dev, /proc, /sys and systemd are understood, enable
  minimal services and then individual hardware-dependent services
  with explicit mocks and filesystem restrictions.
- Start disconnected (\`-nic none\`) with virtual disk only. Do not
  forward real serial/USB, network-accessible management ports,
  host eMMC devices or shared host paths.
- Record kernel config, QEMU version, VM boot log, guest hashes,
  service statuses, requested syscalls/ioctls, and each stubbed
  hardware dependency.

**Phase VM3 — fault injection and comparative testing**:
- Compare 3.6 and 3.10 rootfs execution under the SAME generic
  test kernel and virtual devices, without modifying originals.
- Test invalid signature-file input, missing partitions, corrupted
  uImage on synthetic files, no SD card, missing FDT, filesystem
  out-of-space and interrupted-update handling in a guest-only
  environment. Do NOT supply the real host device nodes.
- Prototype an MPFree initramfs/boot manager, with an explicit
  fallback boot counter and reproducible images for QEMU virt.
- Verify software rollback and copy-on-write reset by deleting
  the disposable overlay and recreating it from the base.
- Record differences *caused by the emulation* as such.

**Phase VM4 — independent U-Boot logic**:
- Build a QEMU-virt U-Boot environment with synthetic boot sources,
  then test MPFree boot script compatibility and update-resilience.
- Separately compare functions to the proprietary U-Boot
  Ghidra evidence; don't assert exact vendor binary bootability.

## Prerequisites and questions for *physical* runtime analysis

**Gate G0 — observe only** (can proceed without bootloader changes):
1. Which MPC port enumerates to USB host in normal operation?
2. Is there a physically accessible UART RX pin, and what is its
   measured I/O voltage and electrical reference?
3. What is the actual complete MMC topology including boot0, boot1,
   user area, RPMB, and the EXT_CSD PARTITION_CONFIG/BOOT_BUS_WIDTH?
4. Does the backup include every user-area LBA and is it internally
   consistent? Have additional eMMC boot regions been imaged?
5. What is Linux bootargs and the runtime kernel's observed
   memory and reserved-memory map? Can this be collected read-only?
6. Which user services invoke bootloader-sig or az01config, if any?
7. Does the existing UART reveal early SPL/U-Boot messages or
   boot input before bootdelay=0?
8. Does a power cycle alone, without a recovery request, change
   USB VID/PID across different boot phases?

**Gate G1 — independent recovery proof, not yet met**:
9. Is genuine immutable silicon ROM USB download mode observable
   independently of executing Akai SPL or U-Boot?
10. Can a harmless ROM USB identification test be performed with
    a known low-risk request and no writing? Which USB connector?
11. Does ROM mode provide a *working* RAM loader path compatible
    with this board's DDR geometry, without writing eMMC?
12. Can an independent recovery environment read all relevant
    boot regions and authenticate a complete offline backup?
13. Has restore of sacrificial storage or an equivalent test
    platform been *rehearsed*, read back and verified to boot?
14. Is any eFuse / OTP / secure-boot policy already active?
    Reading Linux userspace strings is NOT a fuse measurement.
15. Does a power cycle safely exit the tested ROM mode in the
    documented hardware states?

**Gate G2 — only after G1 is proved**:
16. Can U-Boot select mmc1 temporarily with no nonvolatile writes?
17. What is exact U-Boot relocation address, stack/heap,
    fastboot buffer and safe kernel loading region at runtime?
18. Can a deliberately minimal RAM-only owner kernel boot and
    safely return after a watchdog or power cycle?
19. Can an SD-based MPFree kernel and initramfs boot without
    Akai Linux or kexec, while factory boot remains available?
20. Can an independently owned SPL/boot manager be installed and
    restored with a repeatable, reviewed procedure?

No MPFree automation should silently issue reboot-bootloader,
Fastboot download/boot, \`mmc write\`, \`saveenv\`, \`flash\`,
\`program-fuses\`, \`mmc bootpart enable\`, \`mmc extcsd write\`,
or any other hardware-mutating operation.

## Definitions of *recoverable*

- A **VM test** is resettable relative to the preserved disk
  image when no real device/filesystem is shared and the host
  isolation is working. It cannot prove safe physical recovery.
- A **low-risk observational hardware test** avoids intended
  persistent state changes, but can still cause downtime or
  electrical mistakes; use measured pinouts and timeout controls.
- **Bootloader-independent recoverability** requires demonstrated
  silicon-level entry, an independently functional loader, readback
  of all relevant storage, a verified restore procedure, and an
  unchanged immutable trust policy.
- **100% recovery** cannot be guaranteed across power failures,
  flash aging, physical component faults, OTP changes or untested
  boot modes. Never equate a successful QEMU rollback with proof
  that a real MPC One can be recovered after P1/P3 corruption.

## How this advances MPFree independence

Phase 1: real 3.6/3.10 userspace running under QEMU virt => tests
MPFree service and application compatibility independently of
Akai's kernel, and identifies update behavior without risking
hardware. Phase 2: QEMU U-Boot boot-manager tests => robust MPFree
boot script/fallback/rollback implementation. Phase 3: independent
physical ROM recovery proof => safely begin test-loading owner
bootloader code. Phase 4: verified owner SPL+boot manager => boot
does not rely on Akai Linux, kexec or vendor-controlled U-Boot.

The ability to boot Akai Linux userspace in QEMU alone does *not*
achieve the last two phases.
