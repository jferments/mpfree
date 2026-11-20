# Wide, non-destructive MPC One boot-chain audit (November 2026)

This audit investigates **many hypotheses in one run** instead of a sequence
of narrow Ghidra probes.

Run on **ML-tower only**:

~~~sh
bash tools/ghidra/run_wide_bootchain_audit.sh
~~~

Existing inputs:
- Verified regular-file eMMC backup under
  ~/mpfree-backups/mpc-one-emmc-20261109T233604Z-datprr67/mmcblk0.img
  (override with MPFREE_EMMC_BACKUP).
- Analyzed P1 and P3 Ghidra projects under ~/mpfree-work/ghidra/projects
  (override names with MPFREE_P1_PROJECT / MPFREE_P3_PROJECT).
- Ghidra/PyGhidra through installed Snap or GHIDRA_HOME.

Three passes:
1. Parse **offline** P1/P2/P3 from the backup, verify P1 hash and P3 CRC/hash,
   inspect U-Boot environment and, if debugfs is installed, copy P6 into a
   temporary regular file to inspect boot scripts and DTBs read-only.
2. Inspect P1 SRAM SPL's startup, boot-media selection, image loading,
   ARM/Thumb indirect calls, possible ROM return, and memory initialization.
3. Inspect P3 U-Boot's normal boot selection, environment, image validation,
   recovery, Fastboot permission/state machine, storage, and reboot paths.

Private reports are saved in a fresh
~/mpfree-work/bootchain-wide-<timestamp>/ directory:
manifest.json, p1-sram-ghidra.json, p3-uboot-ghidra.json,
storage-env-boot-dtb.json, plus logs.

**Do not publish generated reports:** these may include proprietary
decompilation and private boot-script content. The source code in this repo
does not contain the proprietary Akai binaries.

To skip temporary ~500 MiB P6 extraction:

~~~sh
MPFREE_SKIP_ROOTFS=1 bash tools/ghidra/run_wide_bootchain_audit.sh
~~~

## Confirmed in preceding P3 analysis

- P3 is U-Boot 2016.05-inmusic-20191010, raw ARM code at 0x00100000,
  363649 bytes, SHA256
  b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a.
- Eight Fastboot dispatch entries independently validated.
- Fastboot boot sets a state; its completion routine at 0x1226b4 calls bootm
  at 0x101744. This is a direct call edge.
- Fastboot download, flash, and erase invoke permission helper 0x122834;
  the **runtime** default and RAM load limits are still unknown.
- Legacy U-Boot header and data CRC validation exists in P3's bootm. CRC32
  detects corruption but is NOT a cryptographic signature.
- Image loader recognizes legacy uImage and FDT/FIT formats.
- Candidate download buffer 0x00800800 and U-Boot code base 0x00100000
  alone cannot prove DRAM overlap safety.
- Report image-base metadata 00000000 must not be confused with the verified
  raw payload memory block at 0x00100000.

## Outstanding questions: locations and closure criteria

| Area | Open question | Inspect | Evidence required |
| --- | --- | --- | --- |
| MaskROM | Can software enter **genuine silicon ROM** download? | P1 reset/startup, P3 reboot, DTB reboot-mode | Register writer, reset path, ROM consumer, then safely authorized genuine ROM enumeration |
| ROM upload | Will owner-supplied code execute without Akai's loader? | Public RK3288 ROM protocol, P1 RK32 envelope, eFuse | Owner SPL running with external confirmation (USB ACK alone insufficient) |
| ROM order | What is cold-boot media priority? | P1 eMMC/SD, ID block LBA64, RK3288 boot order | Distinguish immutable ROM order, SPL loop, U-Boot boot_targets |
| SPL handoff | Exactly how does SPL load/enter U-Boot? | P1 0xff705208, 0xff705260, 0xff705350, 0xff7054a4, 0xff7052c0, 0xff705310 | Selected loader mode and proven final indirect address |
| SPL security | Does it verify CRC or signature? | P1 image parser, crypto and eFuse candidates | Exact checks and where they happen |
| Direct boot | Can MPFree boot from SD, USB, or RAM? | P3 bootcmd/distro_bootcmd, MMC/USB drivers, bootm/bootz, P2, P6 script | Reachable non-persistent boot path |
| Boot selection | Is there a RAM-only boot override? | U-Boot environment, bootdelay=0, gpio-keys/serial, boot_targets | Volatile boot-choice mechanism |
| Fastboot | Is download and RAM boot authorized? | P3 0x122834, 0x12289e, 0x122980, 0x122a10, 0x1226b4 | Runtime authorization AND bounded memory map; no active test yet |
| Fastboot | What are continue/boot/error states? | P3 state globals 0x14d658 and callback locations | Complete state transition model |
| RAM | Where are stack, BSS, relocated U-Boot, FDT and buffers? | P1 SRAM, P3 relocation, download and image loader | Demonstrated non-overlap for intended image |
| Recovery | What survives P1/P3 corruption? | True MaskROM, USB OTG ff580000, UART, host readback | Independent eMMC readback via a genuine safe recovery route |
| Security | Is signed boot hardware-enforced? | P1/P3 cryptography, DTB, OTP policy, 3.10 updater | Evidence of silicon enforcement vs unused vendor code |
| Updates | Does newer firmware alter recovery assumptions? | Offline 3.10 update container and rootfs | Partition records and exact updater write targets; no update run |
| Stock OS | Is U-Boot rescue independent from P6? | P2 recovery bootcmd, P3 Fastboot | Path-level verification and limits |

## Strict interpretation limits

Immutable RK3288 MaskROM is silicon code, **not** in P1 or P3.
Reverse engineering these partitions cannot extract MaskROM internals.
The BOOT_BROM_DOWNLOAD value 0xef08a53c is a *candidate* Rockchip reboot
flag, not proof that the MPC One implements it. Its absence from raw binary
words does not disprove code-generated values or indirect behavior.

Raw bytes at BSS and global addresses do not prove actual runtime
values; Ghidra's ARM/Thumb interpretation and indirect flow can be wrong.
Strings and magic values identify leads, not reachability or execution.

Scripts use ONLY an existing verified **regular file** backup and existing
Ghidra projects. No Fastboot, Rockusb, MaskROM entry, rkdeveloptool upload,
erase, flash, eFuse, GPIO, reboot, SSH, or hardware access.

## Next decisions

1. Resolve whether existing U-Boot can directly boot MPFree through a
   non-persistent path. That offers the fastest reduction in startup time.
2. Separately plan safe MaskROM entry and real recovery proof. Start with
   board documentation and passive host-side enumeration, not shorts or writes.
3. Only after independently confirmed recovery consider owner-built SPL or
   U-Boot replacement.

Related: boot-map.md, owner-rom-recovery-plan.md, maskrom-usb.md,
akai-updater-offline.md.


## November 11 owner-run results and second-pass targets

The first owner run (bootchain-wide-20261111T115602Z_3382362)
confirmed **P1 213 functions / 79 relevant ASCII strings** and
**P3 1050 functions / 550 strings** (P3 reached the string cap).
The P2 environment checksum is valid.

**New grounded signals in the owner's uploaded reports:**

- The device tree has PMU reboot-mode register FF730094 with normal
  5242C300, loader 5242C301, recovery 5242C303, bootloader 5242C309.
  U-Boot instructions 00100E4A, 00100EE2, 0010107E, 001010A0
  reference the same register. This DOES NOT prove MaskROM entry.
- U-Boot at 00100FE4 recognizes mode bytes 01 and 09 and sets the
  recovery flag; the board also reads a recovery button ADC channel.
  Full GPIO/ADC mapping and behavior remain unproven.
- U-Boot near 001011E0 constructs the board's fdtfile and publishes
  dorecovery; it also detects the PCB revision.
- Saved P2 boot_targets is mmc0 mmc1, bootdelay 0, scriptaddr 0,
  loadaddr 00800800, kernel_addr_r 02000000, fdt_addr_r 01F00000,
  ramdisk_addr_r 04000000. Investigate U-Boot relocation/overlap
  and scriptaddr 0 before attempting alternative boot.
- P3 U-Boot's legacy uImage header has load 00100000, entry 00000000,
  a 363649-byte payload, and valid CRC32. Investigate how SPL treats
  the zero entry address rather than guessing.
- The P1  ff705208 -> ff705274 Thumb pointer -> ff705350 chain
  is a static connection to a boot-device search loop. ff705310
  executes an indirect device callback and is NOT a proven final
  handoff to U-Boot.
- Fastboot download checks a permission flag; boot queues a bootm
  callback. Runtime authorization state and safe RAM bounds are unknown.
- The MPC 3.10.0 Gen1 update contains rootfs only, with no P1 or P3.
  New boot-related userspace should be compared to the 3.6 baseline.
- The Rockchip BootROM literal EF08A53C is absent from the scanned
  P1/P3 payloads; this is not proof that ROM reentry is impossible.

### Next comprehensive owner run

The runner now invokes MPFreeBootChainForensicsPass2.py against both
saved Ghidra projects. Its targets cover SPL-to-U-Boot, reset mode,
recovery ADC, board revision, U-Boot boot policy, Fastboot permission,
image validation, UART, USB/MMC, memory and SoC-security candidates.

If the extracted 3.10 rootfs exists at
~/mpfree-work/firmware-analysis/MPC-3.10.0-Gen1-update/rootfs.ext4
the runner also runs offline-boot-version-compare.py and includes
differences in boot scripts, DTBs, binaries and services. Set
MPFREE_310_ROOTFS=/absolute/regular-file.ext4 to override.

**Single ZIP output on every run:** the runner calls
package-private-boot-audit.py at exit, gathering all completed JSON,
TXT, LOG, MD, CSV and TSV results into

    ~/mpfree-work/bootchain-wide-<timestamp>.zip

The ZIP contains ZIP_INDEX.json with sizes and SHA-256 for each entry.
The packager verifies the ZIP, then emits a ZIP.sha256 sidecar checksum.
If any optional pass fails, the successful reports and failure logs
are still packaged. No whole-disk backups, firmware binaries, Ghidra
projects, symlinks or unrelated files are included. Keep private.

No static analysis can prove true MaskROM access, a reliable
independent eMMC restore route, or owner-SPL acceptance.


### First-stage DDR and board-descriptor addendum

The P1 SRAM loader's verified text table contains U-Boot SPL 2016.05,
a Rockchip DMC driver, SDRAM parameter and PHY timing property names, an
MMC controller, and UART2. Therefore the new pass also runs
offline-spl-fdt.py on the owner eMMC backup. It looks for an FDT header
inside the verified SRAM-stage payload, validates FDT extents, and uses
dtc *offline* to summarize timing and peripheral configuration. The
output is private p1-embedded-spl-dtb.json inside the single ZIP.

This improves the chances of reconstructing an owner-built SPL's DDR
initialization requirements, but cannot verify real DDR training or
establish that a candidate owner SPL is BootROM-accepted.


## Third pass: full function graph and 3.10 fallback

After the 2026-11-11 owner ZIP, the runner additionally saves
p1-evidence-graph-v3.json and p3-evidence-graph-v3.json. These enumerate
every Ghidra-recognized function's direct calls and nonlocal jump
candidates, plus balanced targeted pseudocode for ROM/reboot, DDR,
SPL, boot media, kernel boot, Fastboot authorization/memory, environment,
security and recovery. Data/address references are tagged as static.

The owner ZIP established a 3,239-byte embedded SPL DTB with DDR controller,
PHY and SDRAM parameters, and showed that the saved P2 environment matches
a candidate compiled U-Boot environment in P3. A cross-stage FDT audit now
also checks for embedded DTBs in P3 and collects boot-related properties.

A prior 3.10 userspace comparison was skipped because its assumed rootfs
path did not exist. The runner now looks in bounded MPFree work folders
for an already extracted ext4 image **verified by the 6.18.44 kernel
listing**. When missing, by default it verifies the known official MPC
3.10 Gen1 update SHA-256, checks for at least 1.3 GiB free, and invokes
the existing **offline only** partition extractor for rootfs. The
extracted proprietary rootfs stays in ~/mpfree-work, outside the ZIP.
The ZIP contains only reports and logs.

Set MPFREE_AUTO_EXTRACT_310=0 to disable automatic *offline* unpacking,
or MPFREE_310_ROOTFS=/absolute/path/to/rootfs.img to specify a previously
extracted image. Neither option contacts the MPC.

The detailed 64-question evidence/backlog matrix is in
bootchain-open-questions-2026-11-11.md. No source-code-only audit can
establish live silicon MaskROM entry, unsigned SPL acceptance, or
independent eMMC restore: these remain explicit physical proof gates.


## V5: avoid repeating saturated V1–V4 Ghidra work

The 2026-11-11T132338Z owner ZIP passed all indexed SHA checks,
and every function presently recognized by Ghidra was processed
(213 SRAM SPL and 1050 U-Boot). Three P3 decompilations failed,
so count them as analyzed/attempted, not recovered pseudocode.

The next investigation has a faster owner-facing entry point:

    python3 tools/mpc-one/run_bootchain_v5_followup.py \
      ~/mpfree-work/bootchain-wide-20261111T132338Z_3414727.zip

It validates the prior private ZIP index, checks every indexed
file SHA256 and both bootloader image hashes, and copies only
necessary JSON reports to a NEW private report directory. It
then runs the new cross-function/indirect-call evidence synthesis,
imports verified 3.10 security/config utilities into EPHEMERAL
Ghidra projects, writes an evidence ledger, and produces ONE new
bootchain-wide-*-v5-*.zip. It does not re-run the 1263-function
static decompilation in P1/P3.

Optional flags:
  --no-elf-ghidra
  --rootfs-310 /absolute/path/to/existing/rootfs.img

The original all-in-one runner also invokes the new V5 analysis
(after its regular passes) for fresh complete audits. New 3.10
ELF helper imports explicitly DO NOT execute vendor binaries.
Their temporary Ghidra projects and ELF copies are deleted once
the private JSON reports have been produced.

V4 evidence now includes a newly named Linux reboot mode
(mode-update) numerically identical to the prior loader mode
(0x5242c301), a new standalone bootloader digest/RC4 utility,
and an early Linux LoadPin unit. None proves genuine silicon
MaskROM access, firmware signature acceptance, SD-first boot,
or independent eMMC recovery. Those remain hardware gates.
