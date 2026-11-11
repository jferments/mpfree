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
