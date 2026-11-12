# MPFree — November 12, 2026, 23:35 UTC integrated boot-chain audit

**Private source:** bootchain-wide-integrated-20261112T233544Z-133891.zip
**Branch:** research/wide-bootchain-audit-20261111

This is an owner-specific, source-only research summary. Never commit
original firmware, full Ghidra pseudocode/ARM dumps, extracted
rootfs/bootloader images or the owner-private report ZIP.

## Integrity and execution conditions

- 107 ZIP members total, including ZIP_INDEX.json; 106/106
  indexed members pass SHA-256 and recorded byte-length verification.
  No extra/unindexed files; ZIP CRC PASS.
- All 15 stages in session-summary.json PASS.
- Broad Ghidra 56.1 s; directed Ghidra 49.1 s;
  overall reported runtime 108.8 s.
- QEMU generic ARM virt guest booted a cloned 3.10 userspace, NOT
  original RK3288 firmware. USB interface 13/13 and endpoint
  21/21 independent synthetic tests PASS.
- No proprietary updater executable, original U-Boot, ROM/SPL, physical
  MPC USB, live ADC, eMMC partitions or OTP was executed/accessed.

## Full stock recovery ADC → Fastboot data path, now pointer-proven

The original U-Boot's recovery ADC test and board setup both resolve
their recovery flag pointer to **0x001462FC**:
- ADC writer's original literal pool at 0x00101154 -> 0x001462FC.
- Board setup's literal pool at 0x001013AC -> 0x001462FC.
- The board setup's environment key pointer reads "dorecovery" and
  value pointer reads "1".
- The SHA-verified saved environment's az01_bootcmd runs
  az01_recovercmd if dorecovery==1, and recovercmd contains fastboot 0.
- Three diagnostic crossrefs identify "Recovery button ADC".
- Original Thumb ARM compares ADC channel-1 reading unsigned <= 99
  and writes a byte value 1 into the same recovery flag.
- The embedded U-Boot FDT enables rockchip,saradc at 0xFF100000.
- Recovery-button logic uses U-Boot ADC device class 10, device 0,
  channel 1. It is NOT the unrelated, unverified JC11S bootcounter
  proposal for register FF7300A0 and magic B001.

This closes the **static** dataflow question from ADC press to
stock U-Boot Fastboot. Physical button identity, actual button voltage
and BootROM fallback remain unverified. Recovery still requires
functioning SPL and U-Boot.

## New SPL indirect boot-selector edge found

The saved P1/SPL original image is SHA-verified by the launcher.
The old direct-call graph could not connect early initialization to
the boot-selection routine at 0xFF705350.

Original image pointer data now exposes:
- 0xFF705274 -> 0xFF705351 (Thumb bit set)
  -> normalized code target 0xFF705350.
- SPL 0xFF705208 dereferences/calls this pointer.
- The boot selector at 0xFF705350 calls media loader 0xFF7054A4
  and dispatches to a second indirect-context helper 0xFF705310.

The second context object pointer is 0xFF70A244. Function
0xFF705310 dereferences callback at object offset +0x0C,
making 0xFF70A250 the next high-value original-image address.
Its *runtime* value and the ultimate second-stage target are
NOT established. The next directed Ghidra pass reads that word and
searches its initializers before interpreting it.

Two other pointer slots:
- 0xFF7053FC -> 0xFF709C70: boot-device order table candidate.
- 0xFF705404 -> 0xFF709C50: device mapping table candidate.

Analysis of selector indexing bounds the mapping table to eight
32-bit words [0xFF709C50 .. 0xFF709C6F], immediately preceding
the five-word boot-device order table beginning 0xFF709C70.
An earlier draft probe incorrectly attempted ten mapping words,
which would have overlapped two order entries. The code and
synthetic tests now use eight, validate the adjacency, and do
not attribute unverified numeric values to SD/eMMC interfaces.

Original ROM/SRAM execution, DDR initialization and independent
whole-eMMC restoration are still separate, unresolved hardware gates.

## Fastboot getvar power-source is the OTHER SARADC channel

The original key literal at 0x00100FB0 resolves to
**"power-source"**. Getvar callback 0x00122D38 delegates
unrecognized standard keys to helper 0x00100EF0, which recognizes
that key, starts and reads **ADC channel 0**, queries its reference
voltage, applies scaling and selects one of four output labels.

By contrast, channel 1 is the physical recovery-button ADC
threshold. This resolves both observed channel uses without
assuming a real measured voltage. Actual category thresholds,
scale factor and four labels still require byte-level literal
recovery. They are included in the next Ghidra probe.

This suggests read-only, authorized physical reconnaissance through
getvar:power-source, but NO such command was sent in this round.

## 3.10 updater linkage: three named code locations are not implementations

The original az01-update ELF report includes:
- usb update mode linkage pointer at 0x00012F98 ->
  named address 0x00014028;
- apply image linkage pointer at 0x00012FAC ->
  named address 0x00014040;
- download image linkage pointer at 0x00012FD0 ->
  named address 0x00014070.

Ghidra's corresponding function records stop at bad-instruction
warnings. Recognition of named linkage/PLT veneers does **not**
prove how the underlying library validates or writes images.

The next V11 phase uses system readelf as a static **data parser**
against the SHA-verified original ARM ELF32 updater and
libaz01-prog bytes. It records required shared libraries,
az01_* imports/exports and R_ARM_* relocations in private JSON.
The integrated comparison checks import/export identity but
still must NOT claim actual dynamic resolution or execution.

The updater's firmware-authentication, failure and storage
commit semantics remain open research questions.

## Other continuing tracks

- USB host interface families FF/42/03 Fastboot-compatible,
  FF/06/05 RockUSB-compatible, and FF/61/7A unidentified
  framed vendor protocol remain distinct; the physical stage
  exposing any interface is not proven.
- Full U-Boot Fastboot table, OEM permission flag,
  gated download/flash/erase and distinct boot→bootm completion
  remain static-confirmed; actual live authorization and safe
  RAM image placement remain unknown.
- 3.10 script header and 463-byte command payload independently
  match the cloned userspace bytes. Both image CRC checks pass.
  This does not prove independent MPFree hardware boot.
- The 3.10 LoadPin verity-digest ioctl and RC4/SHA256
  bootloader digest utility are verified as static program features.
  Actual kernel-policy enforcement or ROM signature requirements
  have not been demonstrated.
- Embedded vendor U-Boot FDT disables all non-eMMC DesignWare MMC
  controllers. The saved mmc0/mmc1 search cannot establish that an
  external SD card provides a working fallback.
- Genuine immutable RK3288 MaskROM enumeration, independent SRAM
  loader acceptance, and complete eMMC restoration remain essential
  recovery gates before modifying early boot stages.

## Implementation after this audit

1. New bounded Ghidra probe captures exact eight-word device mapping
   and five-word order tables, SPL context+0xC second callback,
   and Fastboot power-source scale/labels.
2. New cross-stage synthesizer and synthetic tests distinguish
   a proven FIRST indirect SPL call from an unknown SECOND call,
   and linkage veneers from actual updater implementation.
3. New static ARM ELF relocation parser and tests analyze updater
   imports and shared-library exports without executing binaries.
4. Integrated lab adds both private cross-stage and ELF relocation
   reports to the same SHA-indexed ZIP.
5. The existing recovery ADC, USB, memory, update, SPL, U-Boot and
   MaskROM question matrices continue in the same run; broad
   question count increases to 51 across seven tracks.

All new checks must pass the owner's preflight/regression tests before
the next long Ghidra/QEMU run. A static evidence stage PASS does
not establish physical hardware success.
