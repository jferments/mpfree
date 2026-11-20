# MPFree: verified 2026-11-12 integrated direct-boot archive — findings and follow-ups

Owner-private input: bootchain-wide-integrated-20261112T221627Z-15282.zip.
Source-only public research. Do not commit the owner ZIP, Ghidra
decompilations, raw proprietary firmware, or rootfs images.

## Independent integrity / runtime verification

- ZIP CRC: PASS.
- ZIP manifest: 98 indexed files, 98/98 SHA-256 and sizes verified;
  no unindexed files.
- Total reported runtime: 112.0 seconds.
- Ghidra stages: broad 59.5s and targeted V11 49.1s, both passed.
- V11 decompiled: original U-Boot 99 functions, SPL 44, az01-update
  22 and libaz01-prog 143. The scope is not full firmware call
  coverage; indirect dispatch and hardware initialization remain.
- QEMU generic ARM virt: booted cloned vendor userspace using an
  independently built Linux 7.0.14 guest kernel (NOT RK3288 board
  emulation; NOT execution of vendor U-Boot).
- QEMU synthetic USB model and endpoint cases all passed.
- Focused OEM/static evidence stage passed; P6 compatibility stage
  reported incomplete due to an actual full-image boot script SHA
  mismatch, not due to absent boot assets.

## Key new result: OEM unlock grant writes the same runtime variable read
## by Fastboot download/flash/erase permission helper

The exact original P3 U-Boot source SHA-256 is
b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a.

Byte-level Ghidra literal-pool dereferences:
- 0x0012284C -> 0x0014D690 (permission helper state read).
- 0x00122928 -> 0x0014D690 (OEM grant state write).
- 0x00122924 -> string at 0x00132BAB, 37 bytes, exact published
  authorization-token SHA-256:
  f10b2ec4fa713c1f1d1f584934f501a150119acede06e6012006aa1352e8121c.
- The original Ghidra global xrefs independently show READ at
  0x00122838 and WRITE at 0x0012289E.

The OEM branch at 0x00122854 sets the flag to 1 and responds.
The download handler at 0x00122980 tests permission before accepting a
length; flash (0x00122C5C) and erase (0x00122BF8) separately test it.

**Do not infer** that the unlock is read-only, temporary, persistent,
or always available across bootloader versions. No live OEM command was
sent. First trace any other state writers, startup initialization and
reset lifecycle.

## Fastboot RAM boot: completion has no local permission check,
## but uploading an image remains permission-gated

The Fastboot dispatcher at 0x00122780 uses an eight-entry prefix/callback
table; its *exact command-to-handler mapping* is not yet in the current
saved report. Its boot handler (0x00122964) schedules a completion path
(0x001226B4) which calls bootm (0x00101744) and resets CPU if bootm
returns. No local permission check is visible in these two handlers.
This does NOT establish a locked session can upload a new image:
download (0x00122980) is explicitly permission gated. It also does
not prove a raw zImage passes bootm; ordinary rootfs boot uses bootz.

The next Ghidra probe decodes all eight actual prefix/callback pairs,
including function addresses and Thumb-pointer normalization. A valid
RAM-boot experiment would additionally require image-format/CRC/FIT,
relocation, download-buffer and FDT safety proofs.

## P6 write path: partial safety guards discovered, not safety proven

The original P3 path is flash handler 0x00122C5C -> 0x001100C8
(raw/sparse partition writer) and erase handler 0x00122BF8 ->
0x00110270. These operations use the permission helper.

- Raw writer resolves a named partition and compares an aligned
  requested length against its capacity before invoking a write
  callback through its device driver.
- A sparse-image path at 0x0010FDE8 recognizes chunk types
  0xCAC1, 0xCAC2, 0xCAC3, 0xCAC4 (RAW, FILL, DONT_CARE, CRC32).
- The sparse parser also has a raw/fill partition-end comparison;
  not every malformed/overflow/skip path has been proved safe.
- The erase path resolves a partition and computes aligned erase
  boundaries with a partition-extent condition before its callback.

**None of those observations is permission to flash P6.** Remaining:
full integer-overflow behavior, partition alias resolution, physical
start/end mapping, rootfs flash size cap, sparse skip accounting,
and independent stock image restoration.

## BootROM and bootcount: separate facts

The original P3 word scan finds PMU reboot-mode register FF730094
three times (00100E5C, 00100EE8, 0010114C).
The exact FF7300A0 boot-counter register, B0010000 halfword magic
and EF08A53C ROM download request magic have **no direct 32-bit
occurrences**. This is a negative literal search, not proof that
the corresponding facilities are absent. Computed-address accesses
and other boot stages remain possible. The JC11S openAZ0x counter
report is not hardware proof for the MPC One ACVA.

## QEMU P6 filesystem: actual mismatch and a reporting bug

All four requested files are present in cloned P6:
- /boot/boot.scr.uimg
- /boot/zImage -> zImage-6.18.44-az01-2026-08-10-rt6
- /boot/rk3288-az01-acva.dtb
- /boot/rk3288-az01-acva-c.dtb

Both script wrappers are 535 bytes, but the QEMU clone's script
full-file SHA-256 is
edc7e2c871a2359fa9fe07299d6d70b29671222135f46edeb3eb60026fed14cb,
while the directly extracted 3.10 firmware script SHA-256 is
be713e5e28de35526ea24382c7046a5c4eec3547d295769a44d4d5cad635bdc6.

We have **not established** whether only the header changed or the
executable U-Boot script commands differ. This is why the P6
compatibility stage was marked incomplete.

A transcript parser regex accepted only [a-z_], so it silently
failed to split the 310_trustpoint_unit and 310_policy_assets sections.
The serial log does contain the LoadPin oneshot service:
ExecStart=/usr/sbin/az0x-loadpin-trustpoints,
Before=local-fs-pre.target. Actual kernel policy enforcement was not
tested. Generic ARM virt is running Linux 7.0.14, so its boot should
NOT be confused with booting the vendor 6.18.44 kernel.

## Additional verified static evidence: 3.10 LoadPin and bootloader digest tools

The source archive also contains complete ARM objdump listings (not
executed binaries) for two SHA-pinned 3.10 ELF utilities.

**/usr/sbin/az0x-loadpin-trustpoints**
SHA-256 fd59c2946b2f8e4bc1c143d5270881aa2168ad2851c4726748642d2f4e040239.
The helper calls an ioctl at ARM offset 0x740 with r1 set by
mov/movt to 0x40044C00. This is the Linux UAPI
LOADPIN_IOC_SET_TRUSTED_VERITY_DIGESTS (type 'L', request 0,
argument unsigned int file descriptor). The Linux LoadPin subsystem
reads trusted verity root digests through its securityfs interface.
The source confirms the operation is for configuring trusted verity
digests, NOT necessarily for verifying the kernel before U-Boot boots it.
A running kernel's configuration and enforcement policy remain unknown.

Upstream kernel UAPI reference:
https://codebrowser.dev/linux/linux/include/uapi/linux/loadpin.h.html

**/usr/bin/az01-bootloader-sig**
SHA-256 b7a82030c85ea90e31533773538abfc537d46982bdeaead6963ec9e5ea9a5861.
The decompiled ELF assembly calls RC4_set_key with a 16-byte key
and RC4 with a 512-byte length, together with SHA256_Init,
SHA256_Update, SHA256_Final and a 32-byte memcmp. This is actual
static evidence of a combined RC4/SHA256-related computation,
not a proven RSA verification in the normal U-Boot bootz path.
We have NOT proved which on-disk bytes are opened/compared, whether
the OS invokes this utility at normal boot, or any immutable-ROM
signature enforcement.

These facts are now derived independently by source-only
synthesize_310_security_utilities.py, with strict original SHA
validation and synthetic missing-evidence tests. The next integrated
ZIP will contain security-310-utilities.json and .md.

## New source-only work for the next private round

- Fixed guest section parser to accept digits and added regression.
- Added read-only od of /boot/boot.scr.uimg, legacy uImage CRC and
  multi-image command payload parser, header-only vs command-diff
  comparison, and eight synthetic corruption/format tests.
- Extended SHA-verified V11 U-Boot Ghidra to dereference the Fastboot
  command prefix/callback table. Resolve all eight entries and mark
  exact boot/download callback mapping.
- Increased directed U-Boot Ghidra cap to 140, including raw ARM
  instructions for partition validation, sparse chunk path,
  erase alignment, Fastboot dispatcher and RAM boot.
- Added direct-boot evidence questions on the authorization-to-download
  dependency and partition guards. Now 32 questions in eight
  report-specific topics in addition to the broad 42-question
  seven-track boot-chain matrix.
- Hardened direct-boot evidence PASS to require the new dispatch
  table proof and the existing raw/sparse/erase guard evidence.
- No original vendor executable, Fastboot OEM command, physical
  device, USB bus, storage partition or fuse operation is invoked.

These latest changes require owner-workstation validation; do not
claim their dynamic tests passed based on the previous ZIP.
