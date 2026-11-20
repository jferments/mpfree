# MPFree owner audit — 2026-11-12T22:57:19Z integrated research

Private source: `bootchain-wide-integrated-20261112T225719Z-62363.zip`.
Public repository contains only original MPFree research software and
short derived factual notes. **Do not publish** owner firmware, rootfs,
complete Ghidra bodies, original ARM instruction dumps or this ZIP.

## Chain of custody and execution limitations

- 104 ZIP members, including the manifest; 103 indexed members matched
  their recorded SHA-256 and exact byte lengths. ZIP CRC passed.
- Broad Ghidra 55.4s PASS; V11 targeted Ghidra 49.7s PASS;
  total reported duration 108.4s.
- Generic ARM QEMU guest booted; independent USB synthetic model
  observed 13 interface cases and 21 endpoint cases, both PASS.
- FF/61/7A + ARM return-contract reports PASS; 3.10 security utilities
  source-only audit PASS.
- **One direct-boot stage was marked failed**, not because its command
  dispatcher was missing, but because the Python classifier expected
  `if (uVar4 != 9)` while the original U-Boot uses an equality
  goto-branch `if (uVar4 == 9)`.
- The QEMU script compatibility stage PASSED after BusyBox octal decode.
- No vendor firmware was executed; no physical MPC, USB, eMMC or OTP
  was accessed.

## Important correction: boot-script hashes are of different byte ranges

The original 3.10 update comparison reports:
- `boot_script.firmware310.sha256`: SHA-256 of the **raw 463-byte
  script command payload** (including a final newline), NOT the
  complete 535-byte U-Boot image:
  `be713e5e28de35526ea24382c7046a5c4eec3547d295769a44d4d5cad635bdc6`.
- `header.header_sha256`: SHA-256 of the **64-byte legacy uImage header**:
  `13082a438919f2fb43385caabe9c72346d8284e09756ddce4bc39097d45c1826`.
- `header.image_bytes`: 535, with a canonical two-entry 8-byte
  multi-image script length table and script length 463.

The extracted QEMU cloned P6 uImage passes both header/payload CRC32.
It independently matches **both** the official header SHA-256 and
raw script SHA-256, script length and image structure. The executable
script commands are byte-identical, aside from the upstream
`commands` field dropping a final newline for display. The cloned
complete uImage SHA-256 is
`edc7e2c871a2359fa9fe07299d6d70b29671222135f46edeb3eb60026fed14cb`.

**There was never evidence of a changed boot script.** The previous
`full_image_matches_official=false` was a provenance error caused by
comparing a complete-image SHA with the official report's raw
script-payload SHA. The complete original-image SHA is not provided
under a dedicated key; the corrected code reports direct whole-image
SHA comparison as unavailable, and validates header+payload separately.

## All eight original Fastboot command callbacks remain verified

Table at `0x0012F2B0` has:
`reboot`→`0x00122CF8`;
`getvar:`→`0x00122D38`;
`download:`→`0x00122980`;
`boot`→`0x00122964`;
`continue`→`0x00122948`;
`flash`→`0x00122C5C`;
`erase`→`0x00122BF8`;
`oem`→`0x00122854`.
All callback pointers contain the Thumb bit.

The same original permission global `0x0014D690` is written by
the OEM handler and read by the permission helper. `download:`,
`flash` and `erase` call the permission helper. `boot` completion
invokes `bootm`, but it does not have a direct permission call; the
ordinary image upload path still has a permission gate.

The Ghidra-snapshotted word at the permission global is `0x17`.
**Do not conclude the MPC boots unlocked.** Loaded image bytes do
not establish the value after startup initialization; the original
audit explicitly marks the *runtime* value unknown.

The `getvar:` function formats responses to several string queries
without a local authorization-helper call; it is an attractive
**read-only** hardware reconnaissance path after separate approval.
`continue` and `reboot` also lack local permission checks, but
they are state-changing operations; they are not safe to send as
observation-only probes.

## Reboot and recovery: important second conditional branch

At `0x00100FE4` the source-verified U-Boot code:
- Compares the top 24 bits of a reboot-mode word to its expected magic.
- Checks low-byte values **1** and **9**, directing both to a recovery
  flag assignment, and clears the consumed word.
- **Separately**, calls `FUN_001147F0`, `FUN_00115EF4`,
  `FUN_00115F30` and tests a returned value for **less than 100**.
  When true, it sets the same recovery flag.

That last branch is a verified fact but **not yet identified as a
boot counter**. Its data type, persistence mechanism and physical
meaning are unproven. The JC11S `FF7300A0` boot-counter hypothesis
remains unverified on this original MPC One.

The next directed Ghidra pass has been expanded to analyze the data
source functions, decode exact reboot-mode literal-pool pointers,
and capture further ARM instruction-level evidence. MaskROM entry,
unmodified eMMC recovery and owner-controlled SRAM execution remain
hardware-gated.

## USB, 3.10 update, storage and SPL still tracked

- Host USB libraries continue to expose FF/42/03 (Fastboot-compatible),
  FF/06/05 (RockUSB-compatible) and FF/61/7A (UNIDENTIFIED custom).
  The eight-entry device-side Fastboot table does not prove a physical
  USB port's mode or the presence of ROM USB recovery.
- Normal 3.10 rootfs has the expected zImage, both ACVA device trees
  and 6.18.44 module inventory. This is generic QEMU userspace
  evidence, not an MPFree kernel test on original RK3288 hardware.
- The 3.10 LoadPin helper invokes ioctl `0x40044c00`; the bootloader
  signature utility contains RC4/SHA256 processing. Neither proves
  immutable-ROM enforcement or a currently enabled kernel policy.
- Flash/erase paths have static partition guards, but integer overflow,
  bootloader integrity and independent restoration remain unverified.
- Embedded U-Boot DTB still disables the non-eMMC MMC controllers;
  saved `boot_targets=mmc0 mmc1` does not prove SD-card boot works.
- The SPL indirect handoff and independent ROM recovery gates remain
  separate high-priority studies, not closed by Fastboot findings.

## Source-only changes after this owner archive

- Reworked `verify_uboot_script_wrapper.py` to compare header and
  raw script SHA-256 separately; fixed false whole-image mismatch.
- Corrected reboot-mode equality-branch predicate and discovered the
  separate below-100 path.
- Added precise reboot-mode Ghidra literal-pool probes and targets for
  the opaque counter/data-source helper functions.
- Updated focus to **41 questions, nine topics** and broad coverage
  to **45 questions, seven tracks**.
- Added `recheck_integrated_directboot.py`, a fast, strictly
  SHA-indexed offline source preflight against an existing owner ZIP.
- Expanded tests for byte-range provenance, unrelated metadata-only
  changes, missing source evidence, and reboot-mode branch direction.

These new commits need the owner-workstation regression suite; no
claim is made that new exact committed tests passed before that run.

### Required next-stage questions

1. Determine what U-Boot's recovered value below 100 actually measures.
2. Resolve reboot-mode MMIO pointer, magic prefix and recovery state
   assignments from literal-pool bytes.
3. Trace `getvar:` query dispatch and identify read-only identifiers
   without issuing a device command.
4. Trace Fastboot `download:` memory placement, bootm image type
   constraints and possible alternative RAM upload paths.
5. Verify updater image authenticity/verification and 3.10 policy
   execution independently of generic VM rootfs boot.
6. Resolve MMC driver initialization versus disabled FDT state and
   physical SD boot wiring.
7. Continue immutable ROM entry, independent SRAM loader and whole-eMMC
   recovery analysis. No bootloader or partition writes permitted yet.
