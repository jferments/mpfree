# MPFree — 2026-11-12T23:14:13Z owner boot-chain audit

**Private input:** bootchain-wide-integrated-20261112T231413Z-97183.zip.
Only source-only scripts and small derived facts belong in this repository.
Do not upload the ZIP, original proprietary firmware, rootfs, complete
Ghidra pseudocode, or any full ARM instruction listing.

## Verified integrated run

- ZIP CRC: PASS; 103 of 103 SHA-256-indexed members verified,
  with no unindexed archive members.
- Broad Ghidra 56.5 s PASS; directed Ghidra 49.9 s PASS;
  total 109.7 s.
- QEMU generic ARM virt guest booted; 13 independent synthetic USB
  interface cases and 21 endpoint cases PASS.
- All original integrated stages PASS, including 45-question seven-track
  research, 41-question nine-topic direct-boot evidence, FF/61/7A
  static evidence, ARM return contracts, 3.10 security utilities,
  and 3.10 boot-script compatibility.
- All four P6 boot assets are present. Header (64 bytes) and raw
  script payload (463 bytes) match official 3.10 SHA hashes
  independently; legacy uImage CRCs and command text match.
- No physical MPC, USB device, eMMC, OTP, MaskROM or vendor binary
  was executed or modified in this run.

## Major new finding: recovery button ADC threshold

The owner U-Boot's previously unexplained second condition in
function 0x00100FE4 has been identified as **recovery button ADC**
measurement. This is based on *independent* cross-checks:

1. Error strings in the original P3 and code xrefs:
   - "Recovery button ADC not found" referenced at 0x001010AA.
   - "Failed to start recovery button ADC channel" at 0x001010F0.
   - "Failed to read recovery button ADC channel" at 0x00101100.
2. Original ARM calls:
   - 0x001010A4 calls 0x001147F0 to get U-Boot device-class 10,
     device index 0.
   - 0x001010EA calls 0x00115EF4 with channel argument 1.
   - 0x001010FA calls 0x00115F30 to read that channel.
   - 0x00101106 calls 0x00115ED8 after a successful read.
   - 0x0010110C compares output to 0x63 (99); at
     0x0010110E–0x00101114 the ARM IT/LS sequence conditionally
     stores a byte value 1 to the recovery flag if unsigned <=99.
3. The original embedded U-Boot device tree aliases
   adc0 = "/saradc@ff100000", and declares an active
   rockchip,saradc device at 0xFF100000 with status="okay".
4. Older U-Boot driver-model source gives UCLASS_ADC numeric
   value 10 for an appropriate historical enum variant.
   This is corroboration, **not exact version-match proof**.
5. U-Boot function 0x00100EF0, reachable through Fastboot getvar,
   also accesses class 10, index 0, but uses ADC channel 0.
   Its exact getvar key and interpretation of the returned value
   have not yet been resolved.

This is a genuine stock U-Boot **analog button recovery trigger**,
NOT proof that the related JC11S bootcounter register
0xFF7300A0 or B001 magic is used on the MPC One. The bootcounter
hypothesis remains independent and unverified.

Source code references:
- https://android.googlesource.com/platform/external/u-boot/+/f7aa2094d4b1f8121384a030e95177327d873339/include/dm/uclass-id.h
- https://github.com/rockchip-linux/u-boot/blob/next-dev/arch/arm/mach-rockchip/boot_rkimg.c

## Recovery flag → boot environment: partly established

At 0x00100FE4, both a recognized reboot-mode request
(0x5242C301/0x5242C309; register 0xFF730094) and recovery ADC
value <=99 store 1 in a recovery flag accessed via
a literal at 0x00101154, resolving to 0x001462FC.

Another U-Boot board-setup function at 0x001011E0 checks
a recovery flag through the pointer literal at 0x001013AC
and calls its environment setter for key "dorecovery" if set.
The saved az01_bootcmd tests dorecovery == 1, and
az01_recovercmd contains "fastboot 0".

**Still to prove:** 0x001013AC must be dereferenced and shown
to reference exactly the same flag address (0x001462FC)
as the ADC/reboot-mode writer. The next Ghidra pass captures
both pointer values, the ADC diagnostic string pointers,
and the getvar ADC key. The new integrated stage fails closed
if literal-pointer equality is not independently established.

We have not verified the physical button identity, the real ADC
value range or how a user holds it during power-on. The stock
recovery path requires functioning SPL and U-Boot; it does NOT
constitute immutable MaskROM recovery.

## Other boot-chain tracks remain active

**SPL:** Candidate first-stage code and boot-device selection are
captured, but ROM entry acceptance and independent RAM loader
execution are unverified. Next Ghidra probes decode the raw SRAM
indirect-stage handoff pointer and boot-media table addresses.

**U-Boot/RAM boot:** The complete eight-entry original Fastboot
table, the OEM authorization flag at 0x0014D690, the permission-gated
download/flash/erase handlers, and the distinct boot→bootm
completion are static-confirmed. RAM download memory ownership
and accepted image formats are NOT tested on hardware.

**3.10 updater:** az01-update has distinct USB update, apply-image,
erase-data and download entrypoints, but those wrappers defer to
dynamic library targets. Next Ghidra probes record relevant
trampoline original-image words and their candidate targets;
this is not original code execution or a verified runtime binding.

**USB:** Host FF/42/03 Fastboot-compatible, FF/06/05
RockUSB-compatible, and FF/61/7A **UNIDENTIFIED** custom-framed
interface remain separate. None is authenticated as physical
silicon MaskROM USB.

**Firmware security:** The 3.10 LoadPin helper's verity-digest
ioctl and the bootloader-sig utility's RC4/SHA-256 routines
are confirmed statically. Kernel enforcement, updater verification
and immutable ROM security fuses remain unverified.

**External-media boot:** Original embedded U-Boot FDT enables
eMMC but disables the three other DesignWare MMC nodes. The
saved mmc0/mmc1 scan order alone does not prove usable SD boot.

**Recovery:** Neither independent MaskROM enumeration, accepted
owner SRAM loader, nor independently verified complete eMMC
readback/restoration has been achieved.

## Research tool changes

- New offline synthesize_recovery_adc.py: cross-validates diagnostic
  xrefs, exact ARM conditional code, SARADC FDT and
  board_setup→dorecovery→saved Fastboot evidence. Pointer equality
  is reported separately, never guessed.
- Synthetic tests including negative diagnostics, ARM branch,
  ADC disabled in FDT and mismatched pointer fixtures.
- Ghidra V11 targeted pointer work on ADC, board_setup, getvar
  labels, SPL indirect handoff and updater trampoline slots.
- Single integrated run packs private recovery-adc-evidence JSON/MD
  and separates this stage from the Fastboot/USB and original
  seven-track matrices.
- Existing-ZIP preflight verifies the static evidence without
  requiring new pointer data from its older Ghidra script.
- Broad research matrix now contains 47 questions; focused
  matrix contains 42 across ten topics, replacing the old
  speculative threshold interpretation with recovery-button ADC.

New tools still require a fresh owner workstation run. Do not treat
source-only hypotheses as physical recovery verification.
