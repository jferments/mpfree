# MPFree owner boot-chain research — integrated 2026-11-13T00:33:41Z

Private owner archive: bootchain-wide-integrated-20261113T003341Z-208183.zip
(the upload may have a local "(1)" suffix). Original firmware, rootfs,
vendor binaries, machine-code dumps, complete Ghidra pseudocode and
the owner ZIP MUST remain private. This document contains only
high-level facts derived from SHA-indexed original evidence.

## Archive verification and execution limits

- 112 ZIP members total including ZIP_INDEX.json; 111/111 SHA-256
  indexed members match both digest and size, CRC passed,
  no unindexed members.
- All 17 integration stages passed. Broad Ghidra 75.8 s, directed
  Ghidra 48.2 s; QEMU booted cloned 3.10 **userspace** on generic
  ARM virt, not a native RK3288 or original vendor bootloader.
- Independent USB synthetic tests passed 13 interface and
  21 endpoint tests; they do not exercise the original host USB
  library against the physical MPC.
- Existing focused audit: 42 questions across ten topics.
  Existing broad audit: 51 questions across seven tracks.
- No physical MPC, /dev/mem, USB bus, eMMC, BootROM, fuse/OTP or
  original vendor executable was executed or modified.

## SPL: first indirect call resolved, second original word is zero

Original P1 SPL SHA-256:
6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f.

- Original indirect-call slot FF705274 -> FF705351, Thumb function
  FF705350, which is SPL boot-selection loop. Caller FF705208
  invokes that pointer. This is a **static** local SPL code edge,
  not proof of immutable-ROM acceptance.
- Boot selector calls helper FF705310 with a context pointer
  FF70A244. Helper calls context +0x0C, slot **FF70A250**.
- The verified original-image word at FF70A250 is **0x00000000**.
  That is not a valid known stage target. It requires tracing
  runtime initialization or another nonexecuted control path.
  Zero in the saved image does NOT mean the actual CPU calls 0.
- New Ghidra focus: pinned boot-source resolver FF704120 and
  first-entry writer FF705342, direct xrefs to table/callback
  slots, original bytes of mapped names, and the second
  callback's initialization candidates.

## SPL: original boot-order table is NOT a live priority list

- Boot-order table begins FF709C70, five 32-bit slots.
  **All five original words are 0xDEADBEEF**.
- The selector calls FF705342 which obtains a boot-device
  value through FF704120 and writes it into table slot 0
  before looping. The other entries and physical priority
  require runtime dataflow/initialization analysis.
- Eight-word mapping table FF709C50..FF709C6F holds:
  ID=1 -> string at FF709A8F "MMC1";
  ID=2 -> FF709A94 "MMC2";
  ID=3 -> FF709A99 "MMC2_2";
  ID=0xDEADBEEF -> FF709AA0, label not independently
  identified.
- The mapping names are confirmed by independent P1 Ghidra
  string evidence, NOT evidence of board physical wiring or
  successful SDMMC boot.
- Embedded stock U-Boot FDT still reports non-eMMC MMC
  controllers disabled. Saved mmc1 target is no proof of
  external boot.

## Fastboot power-source ADC now has exact thresholds

- Original U-Boot getvar key is "power-source", uses SARADC
  channel 0. Channel 1 is the recovery button, and <=99 sets
  the shared `dorecovery` flag leading to stock Fastboot.
- Verified response strings:
  "invalid", "battery", "ac", "invalid-ac".
- Direct literal thresholds on the calculated uint32 value:
  0x00098967 = 624999;
  0x0010A1CF = 1089999;
  0x001312CF = 1249999.
- For ordinary nonnegative signed-range outputs:
    <=624999          -> invalid
    625000..1089999   -> battery
    1090000..1249999  -> ac
    >=1250000         -> invalid-ac.
- The U-Boot expression scales an ADC-channel reading times
  reference value with fixed-point multiplier
  0x10624DD3 = 274877907, followed by >>38.
  Multiplier / 2**38 is almost exactly 1/1000.
- The code mixes unsigned comparison at first bound and
  signed casts at later bounds; the new source-only model
  preserves this distinction for out-of-range uint32 values.
- No physical ADC sample/reference scale or voltage unit has
  been verified; threshold numbers must NOT be advertised
  as measured volts or host hardware specifications.

## 3.10 updater ELF/crypto verification pipeline

Original SHA-verified:
- az01-update: 57511e1d503f3ceaae3cb9bc40174e073b7ae410e65c986f1991731c77ae655b.
- libaz01-prog: af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623.

The updater ELF imports and libaz01-prog exports match for
az01_usb_update_mode, az01_apply_image and az01_download_image.
The updater also imports download_init/cleanup, erase_data_partition
and strerror. These are linked-library API identities, not proof
of execution or shared-object binding at runtime.

High-confidence original Ghidra library static calls:
- Download path sets onboard-image key lookup, runs
  az01_verify_image(...,0), and conditionally reaches
  az01_apply_image after an intermediate callback check.
- az01_write_image invokes device key lookup and
  az01_verify_image(...,2), then conditional image
  compatibility checks, then az01_prepare_to_write and
  partition iteration callbacks.
- az01_verify_signature calls EVP_sha256,
  EVP_DigestVerifyInit and EVP_DigestVerify with a signature
  **length argument 0x100 = 256 bytes**. Key type/accepted
  certificate roots and actual verification policy are not
  established by these calls alone.
- az01_verify_image uses cached verification-state values
  (including state 1 and state 2) and an option/flag check.
  The presence of verification code does NOT establish
  that all control paths run cryptographic verification.
- az01_commit_bootloader dispatches backend callback
  slot +0x2C; write_bootloader uses +0x18;
  write_partition uses +0x1C.
  Concrete USB transport callback selection and all
  error/commit semantics remain open.

Importantly, this cryptographic userspace policy must not be
confused with U-Boot FIT/legacy bootm checks, 3.10 LoadPin
verity-digest ioctl, or silicon BootROM secure-boot fuses.

## Three USB backend callback tables differ

The original imported libaz01-prog ELF/Ghidra table addresses
have a 0x10000 Ghidra load bias. Existing source-only
USB descriptor/class/endpoint findings remain:
- FF/42/03 Fastboot-compatible, table 0x00012A74:
  write_bootloader, write_partition, stream_partition,
  commit_bootloader and reset callbacks present.
- FF/61/7A *UNIDENTIFIED* custom vendor framed protocol,
  table 0x00012B48:
  write_bootloader, write_partition, stream_partition
  and reset callbacks present; commit_bootloader slot NULL.
- FF/06/05 RockUSB-compatible, table 0x00012BD8:
  write_bootloader, write_partition and reset present;
  stream_partition and commit_bootloader slots NULL.

All table callback addresses correspond to Ghidra-recognized
library functions at (ELF vaddr + 0x10000). The selector
initializers reference the named tables.

This is static **backend API shape**, NOT proof that a live
physical interface operates or that missing callbacks rule out
alternate protocol operations. Do not identify FF/61/7A
as RockUSB, and do not equate stock Fastboot with silicon
MaskROM.

## No new physical recovery or safe install proof

- Rootfs P6-only replacement preserves P1/P2/P3 but replaces
  the stock userspace; no reversible dual boot is proven.
- Working stock U-Boot ADC→dorecovery→fastboot is
  source-confirmed. It requires usable SPL/P3.
- Actual MPC RK3288 BootROM USB enumeration, owner SRAM
  loader acceptance, full physical eMMC restore, and eFuse
  security state remain unverified.
- Fastboot RAM download size (advertised max 0x70000000)
  is not free RAM. Bootm image format, relocation/FDT
  overlaps and authorization must be resolved before
  owner-initiated physical RAM-only experiments.
- No vendor firmware execution, eMMC writes, flash/erase
  commands or hardware USB capture occurred.

## Changes after this archive

- New source-only `synthesize_multitrack_followups.py`:
  exact SPL placeholder/order/label checks, original
  ADC threshold/scale arithmetic, and updater
  image verification, signature helper and backend
  write callback evidence.
- New source-only `synthesize_usb_backend_coverage.py`:
  exact three-table callback slot comparisons against
  distinct Ghidra functions and source SHA hashes.
- Added 12 synthetic positive/negative tests plus new
  source guards; original Ghidra probe expanded with
  bounded SPL table strings/xrefs and boot source resolver.
- Integrated runner now packages both reports, alongside
  all original 17-stage evidence; expected two more
  stages in next run, if no other change.
- Seven-track question matrix expanded **51 -> 59**;
  focused ten-topic matrix remains **42**.
- Owner ZIP preflight rechecks 59-question matrix and
  both new analysis modules without Ghidra/QEMU.

These new tests and Ghidra probes must run on the owner
workstation before claiming the updated stages pass.

## Next outstanding proof priorities

1. Trace runtime initializer/writers for SPL context callback
   FF70A250, whose saved word is zero.
2. Trace first and remaining boot-order slots away from
   the original DEADBEEF values, and map MMC1/2/2_2 to
   real RK3288 controllers and wiring.
3. Establish physical units of ADC channel 0 and test
   read-only power-source response only after explicit
   authorized device connection.
4. Trace actual image verification dispatch and the
   cached-state/flag behavior in libaz01-prog v3.
5. Resolve concrete write/commit implementations for
   each USB backend, including device error propagation
   and whether a storage operation can be repeated safely.
6. Keep independent MaskROM, SRAM loader and complete
   eMMC restoration evidence separate from stock Fastboot.
