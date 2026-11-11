# MPFree: comprehensive open-question inventory after bootchain-wide 20261111T123312Z

**Evidence boundary:** Everything here is based on the owner's offline SHA-validated
P1 SRAM SPL, P2 U-Boot environment, P3 U-Boot executable, embedded device
trees, P6 stock rootfs, and original 3.10 update-container inventory.
The immutable RK3288 silicon BootROM was **not** extracted, executed or
captured. No active MaskROM / recovery / Fastboot RAM-boot test has been
performed. Values described as candidates are not proven runtime state.

## New facts from the owner's second ZIP

- The 19-report ZIP includes validated P1 and P3 wide audit and forensics
  Ghidra results. P1 has 213 Ghidra-recognized functions, P3 1050.
  Third-pass forensics decompiled 38 P1 and 127 P3 functions.
- One embedded **3,239-byte DTB** starts at SRAM SPL offset **0x627c**
  (25212 bytes), image SHA256:
  98b8bdd196666f6064fd26ddf950857f576a6c581c9b0d4f9550e42d57c3a906.
  It contains rockchip,pctl-timing, rockchip,phy-timing,
  rockchip,sdram-params, DMC register blocks 0xff610000..0xff640000,
  UART2, eMMC and early-stage clock/pin configuration.
- The P3 payload contains a candidate compiled-in U-Boot environment at
  payload offset **208903**. All compared boot-related variables match
  the valid saved P2 environment, including bootcmd, scan logic,
  boot_targets, bootdelay, kernel/FDT/initrd addresses and recovery command.
  This is strong default-env evidence but not runtime behavior.
- The board's PMU reboot-mode register is **0xff730094**. Device-tree
  values: 0x5242c300 normal; 0x5242c301 loader; 0x5242c303 recovery;
  0x5242c309 bootloader. U-Boot uses 0x01 and 0x09 to set recovery.
  No Linux DTB maskrom value; no literal 0xef08a53c in P1/P3.
- U-Boot's recovery button ADC code accepts a value less than 100 from
  channel index 1. GPIO/ADC board mapping and trigger timing remain open.
- The P1 loader parses legacy uImage with two modes. With the observed
  P3 entry field 0, the default apparent mode uses the *load address*
  (0x00100000) and buffer at (load - 64). The actual mode flag and
  final branch need proof.
- The P3 Fastboot download accepts sizes 1..0x70000000 in its current
  code path, but the receive destination is 0x00800800 plus a received
  byte count. This huge limit is **not a safe RAM availability bound**.
- A Fastboot permission check gates download. The OEM command routine
  contains a permission-variable write; exact initialization/authorization
  remains unclear. Do not infer unlocked state from raw file bytes.
- Stock boot preferences include mmc0 then mmc1. U-Boot Fastboot recovery
  is triggered by the board flag. The 3.10 official image contains only
  rootfs, not new P1/P3 bootloaders.
- The 3.10.0 **rootfs comparison was skipped**: the launcher looked for
  the extracted rootfs at one hardcoded path that did not exist.
  The third pass includes a bounded offline locator.
- The Linux reference DTB has **two different eFuse regions**:
  secure eFuse at 0xffb10000 and normal eFuse at 0xffb40000.
  Absence of direct MMIO xrefs does not prove the fuse state.
- Public upstream U-Boot documentation describes RK3288 first-stage
  BootROM handoff and potential return-to-ROM. More recent generic
  Rockchip work on two-stage USB RAM boot is not automatically valid
  for this old RK3288 board.

## Questions and exact next-evidence tactics

### A. Silicon MaskROM, SRAM USB protocol, recovery independence (Q01–Q10)

| ID | Open question | How next offline pass helps | Hardware proof still needed? |
|---|---|---|---|
| Q01 | Can software force *genuine silicon* MaskROM with a valid eMMC loader? | Cross-reference reset mode register, P1 source selection and P3 reboot handler; distinguish 0x5242c3xx from 0xef08a53c | Yes, genuine Rockchip enumeration |
| Q02 | Does first-stage code return control to ROM under a recognizable condition? | Full P1 callgraph, relevant computed call/jump sites, reset and failure paths | Yes |
| Q03 | Which physical connector is ROM USB OTG on this MPC One PCB? | Compare SPL, U-Boot, Linux embedded DTBs and ff580000 device configuration | Yes, cable and VID/PID observation |
| Q04 | Is the Rockchip 0x471 / 0x472 SRAM/DRAM two-stage USB mechanism available on RK3288? | Cross-check P1 format with upstream RK3288 vs later Rockchip docs; no extrapolation from RK3588 | Yes, once true MaskROM proven |
| Q05 | In which order does silicon probe NAND/eMMC/SD/USB, as distinct from SPL and U-Boot order? | Compare public RK3288 ROM source with P1 device loop and P2 boot_targets | Board-specific validation |
| Q06 | Could SD become an independent recovery medium without destructive eMMC changes? | Inspect boot path/fallback and board pin topology without changing boot targets | Hardware design required |
| Q07 | Would the silicon accept a newly built and unsigned SRAM loader? | Extract exact RK32 envelope, DDR parameters, image/CRC/signature checks | Yes, non-destructive execution probe |
| Q08 | What is the maximum usable SRAM stage for this vendor's boot pathway? | Validate ID-block geometry, 32-KiB decoded stage, symbol/callgraph footprint | Yes, if repacking |
| Q09 | Is USB Rockusb loader mode distinct from Akai Fastboot on physical USB-B? | Correlate driver/controller descriptions and different protocol/VID/PID expectations | Yes |
| Q10 | What survives total corruption of P1/P2/P3? | Produce dependency chart of Akai U-Boot Fastboot vs immutable ROM recovery | Yes, independent readback/restore test |

### B. SPL, memory and second-stage handoff (Q11–Q21)

| ID | Open question | How next offline pass helps |
|---|---|---|
| Q11 | Precisely which P1 routine is first executed after BootROM's SRAM handoff? | Model 0xff704020 entry and ARM/Thumb mode and first stack |
| Q12 | Is ff705350 a five-entry priority list, and which media does each entry represent? | Trace P1 bootdev table values and MMC callback metadata, not just source strings |
| Q13 | Does SPL retry the same medium before falling back? | Trace loop-control and early failures to media drivers |
| Q14 | Which LBA/length is actually read for P3 uImage on each medium? | Trace ff7054a4 storage-callback arguments and P3 header byte counts |
| Q15 | Which of the two uImage parser branches is selected at boot? | Track selector and flag at DAT_ff70530c[5], not just decompiler pseudocode |
| Q16 | What is the *actual* loaded image entry address when the uImage field is zero? | Resolve descriptor record[2..4], ff705310 indirect call, and stage handoff state |
| Q17 | Does SPL check P3 CRC32, format magic, lengths, RSA signatures, or none? | Follow image-parse callees and callsites before handoff |
| Q18 | What is the full DDR init sequence and topology? | Cross-stage FDT extraction and references to rockchip,pctl-timing/phy-timing/sdram-params |
| Q19 | Is the vendor SPL source-compatible with an upstream 2016-era RK3288 SPL? | Compare driver names/call graph/DTB fields to publicly available U-Boot |
| Q20 | Which stage owns UART2 and does it output early recovery errors? | Identify UART register 0xff690000 init/log calls and pin choices |
| Q21 | What can cause an SPL reset/recovery loop rather than a hard brick? | Inspect boot-failure functions, watchdog/reboot calls and retry branches |

### C. U-Boot startup, environment, boot choice and SD/USB fallback (Q22–Q33)

| ID | Open question | How next offline pass helps |
|---|---|---|
| Q22 | Which routine chooses recovery after reboot-mode 01/09? | Analyze 0x00100fe4 direct callers, 0x001011e0 env update and PMU write xrefs |
| Q23 | Which ADC channel and board input triggers recovery? | Trace channel index 1, threshold 100, ADC driver, U-Boot embedded DTB |
| Q24 | How is PCB revision derived and which DTB is chosen? | Analyze 0x001011e0 GPIO read and fdtfile construction against actual board DTBs |
| Q25 | What happens for reboot flags 0x5242c303 (recovery) and unknown values? | Reconstruct complete switch/if ladder and state transitions, not just strings |
| Q26 | How and where does P2 environment CRC failure select compiled defaults? | Follow env read/parse/CRC call graph and matching embedded defaults |
| Q27 | Can boot order be influenced transiently through environment without persistent flash edits? | Identify U-Boot command inputs and RAM-only variables; no live command issued |
| Q28 | Under what condition does U-Boot attempt mmc1 after mmc0? | Model boot script scan, successful bootz non-return, and failure branches |
| Q29 | What extlinux features are truly compiled and reachable? | Resolve sysboot/pxe command registration and supported image/FDT syntax |
| Q30 | Is direct USB mass-storage boot present, not just USB Fastboot? | Trace USB host init/scan command registrations against environment |
| Q31 | Is source/scriptaddr=0 safe with current U-Boot relocation? | Analyze source command loader plus actual memory map; report overlap assumptions |
| Q32 | Does the board allow an interactive U-Boot console despite bootdelay=0? | Trace serial/gpio-keys stdin and autoboot interruption callbacks |
| Q33 | What causes reset_bootloader when boot scripts fail, and does it always reach U-Boot Fastboot? | Trace command registration, PMU register write, reset path |

### D. Fastboot authorization, RAM boot, images and memory (Q34–Q46)

| ID | Open question | How next offline pass helps |
|---|---|---|
| Q34 | Exactly which routine initializes Fastboot permission flag at 0x0014d690? | Collect ALL readers/writers and the early initialization/BSS memset path |
| Q35 | What distinguishes permission check, OEM command, and authorization state transition? | Trace permission writer and OEM dispatcher; redact credentials |
| Q36 | Does permission persist across reboots? | Compare permission flag BSS/relocation path and PMU state; runtime test later |
| Q37 | What are Fastboot download protocol sizes, chunk sizes, and callbacks? | Trace complete dispatch, callbacks, global counters and error paths |
| Q38 | Is there an actual safe maximum download length distinct from 0x70000000? | Map 0x00800800 buffer, U-Boot relocation, stack, BSS, FDT, image relocation |
| Q39 | Can bootm consume the received RAM image without an extra translation or signature? | Trace boot-done -> bootm -> image classification/verification chain |
| Q40 | Does Fastboot boot expect legacy uImage, Android boot.img or FIT image? | Trace image classifications, bootm arguments, and image-header validation |
| Q41 | Which FIT and legacy hash algorithms are enabled? | Enumerate FIT hash routines and legacy CRC32 path, distinguish public digest from signature |
| Q42 | Is bootz available through a direct U-Boot environment/script route? | Identify command table and actual bootz entry vs bootm wrapper |
| Q43 | Does Fastboot continue resume normal boot or only leave USB gadget state? | Full USB exit-state setter/read caller graph |
| Q44 | How is the USB gadget attached/detached and can it recover after USB errors? | Trace gadget driver and reset/continue error paths |
| Q45 | What happens when bootm returns on a malformed downloaded image? | Trace boot-done fallback reset, error response and watchdog |
| Q46 | Are read-only Fastboot getvar commands available without authorization? | Trace dispatch gating independently from download/flash/erase |

### E. Security, eFuse, integrity, firmware updates and owner autonomy (Q47–Q56)

| ID | Open question | How next offline pass helps |
|---|---|---|
| Q47 | Is the *silicon* secure boot fuse actually enabled? | Distinguish ffB10000 secure and ffB40000 normal efuse; static cannot prove runtime bit |
| Q48 | Does P1 or P3 enforce cryptographic signature verification before owner code? | Trace actual verification path; avoid overinterpreting CRC and strings |
| Q49 | Is the FIT SHA256 machinery generic code or part of enforced verified boot? | Graph FIT hash invocation and failure-to-boot edges |
| Q50 | Does 3.10 add bootloader-signature verification in userspace only? | Locate extracted 3.10 rootfs, hash/diff new binary and service |
| Q51 | Can firmware 3.10 modify P1/P3, fuses or hardware even though the container holds only rootfs? | Enumerate startup services/updater behaviors and peripheral firmware update routes |
| Q52 | Is official update-image authentication performed by RAM updater or U-Boot? | Compare updater source paths and executable crypto linkage; no update execution |
| Q53 | Is rollback protection or anti-downgrade enforced? | Inspect versioning/compatibility predicates in updater/bootloader, not just version strings |
| Q54 | What is the factory rescue dependency of P6 vs P1/P3? | Draw static dependency graph and enumerate failure partitions |
| Q55 | Can a future independent boot path avoid proprietary signatures while preserving original firmware? | Map viable RAM-only boot and planned ROM-owner SPL options, not flash now |
| Q56 | How could we verify recovery after corruption *without deliberately corrupting storage*? | Stage independent ROM read-only access and compare complete backup hashes first |

### F. Runtime board behavior and cross-version reproducibility (Q57–Q64)

| ID | Open question | How next offline pass helps |
|---|---|---|
| Q57 | Why did the 3.10.0 comparison not run? | Offline rootfs candidate locator searches bounded MPFree work folders; verifies actual 6.18.44 kernel |
| Q58 | Which 3.10 DTB properties differ in reboot-mode, USB, power and pins? | Compare extracted 3.10 device-tree nodes to installed 3.6 DTBs |
| Q59 | What Linux command-line/DTB constraints would affect an MPFree direct boot? | Compare P2 env, P6 boot script, kernel/FDT parameters and memory reservations |
| Q60 | What is the exact eMMC and SD numbering in SPL/U-Boot/Linux? | Cross-reference early SPL DTB, U-Boot DTB, P6 Linux DTB and MMC drivers |
| Q61 | Can a read-only host-side USB enumeration distinguish standard/Update/MaskROM modes? | Draft guarded identification-only procedure, no USB download; requires owner opt-in later |
| Q62 | Can UART be observed safely and passively before attempting alternative first-stage loaders? | Trace serial2 115200 and board pin mux requirements; physical pinout remains separate |
| Q63 | What outputs from this analysis are still missing or truncated? | Track all analyzed function counts, source truncation, missing DTBs, failure logs and skipped stages |
| Q64 | What is the minimum non-destructive proof required before writing any early-boot partition? | Evidence gates: verified backup, genuine ROM entry, independent readback, owner code execution, tested restore |

## Next automation: how to maximize discovery in one run

- **Full function graph**: export all direct-call edges from every recognized P1
  and P3 function (not just 127 chosen functions).
- **Balanced follow-up**: take all explicit SPL/ROM, U-Boot boot, Fastboot,
  security, memory, env and hardware anchors, plus their callers and callees,
  so address ordering cannot starve high-address research areas.
- **Source/sink audit**: classify references to PMU reset, UART, efuse
  0xffb10000/0xffb40000, Fastboot permission and byte-count variables,
  and P1 parser descriptor fields; include disassembly windows.
- **Embedded FDT cross-stage comparison**: recover validated DTBs from SPL
  and U-Boot raw payloads, compare DDR and boot-critical properties and
  map those against the Linux DTBs already extracted offline.
- **3.10 rootfs recovery**: locate an existing, uniquely kernel-verified
  3.10 rootfs or explicitly report why it is missing. No network, download
  or update execution.
- **Reasoned ranking**: report the top gaps that require hardware
  observation and cannot be closed through repeated static disassembly.
- **Private archival**: one ZIP with ZIP_INDEX.json and SHA256 checks;
  include every successful result and failure log, exclude raw firmware,
  whole eMMC images, private keys, and Ghidra project files.

## Stop criteria (avoid infinite reverse-engineering loops)

If a domain has no further Ghidra xrefs, no new instructions, no library
symbol leads and no public SoC/board source, label it as **hardware
observation required** and do not imply another string-scanning script will
prove it.

Never intentionally erase eMMC, short pins, program OTP, flash bootloader,
modify P2 environment or request true MaskROM as part of this audit.


## Third owner ZIP: updated findings and Q65–Q88

Input: bootchain-wide-20261111T125321Z_3403565.zip (31 ZIP entries,
all major V3 Ghidra passes and the firmware-3.10 comparison succeeded).
This appendix supersedes the older "3.10 comparison skipped" note.

- P1 has 213 recognized functions, 86 V3 decompilations, 127
  recognized functions without V3 pseudocode. P3 has 1,050 recognized
  functions, 240 V3 decompilations and **810 without V3 pseudocode**.
- U-Boot embeds a **35,213-byte DTB at payload offset 328436 (0x502F4)**.
  Its rockchip,pctl-timing, rockchip,phy-timing, rockchip,sdram-params
  arrays match the SPL's 3,239-byte embedded DTB exactly.
- U-Boot DTB aliases identify mmc0 -> ff0f0000 (non-removable eMMC)
  and mmc1 -> ff0c0000 (removable SD). The 3.10 Linux DTB independently
  supports that physical distinction. Boot targets mmc0 then mmc1
  make SD *fallback*, not a user-controlled primary boot mode.
- The previous extracted 3.10 rootfs is present and was located at
  ~/mpfree-work/akai-unpacked-20261110T024913Z-1hjja3_r/rootfs.img,
  size 451468288, containing zImage-6.18.44-az01-2026-08-10-rt6.
- SHA differences occurred in boot.scr.uimg, two target DTBs,
  az01-update, az01-launch-MPC, az01-script-runner and
  libaz01-prog.so.1.3; fw_env.config did not change.
  **Executable boot script commands match** after stripping a
  trailing newline; a changed uImage header is not evidence of
  changed boot commands.
- 3.10 az01-bootloader-sig references MMC partitions 1, 2 and 11,
  SHA256_Update, and primary/fallback bootloader hash mismatch.
  Presence in 3.6 was NOT checked by the earlier script and remains
  unknown until the V4 cross-version binary audit.
- The 3.10 libaz01-prog contains names for write_bootloader,
  commit_bootloader, verify_image, verify_signature and OpenSSL EVP
  operations. These describe possible updater behavior, NOT proof
  that the official 3.10 rootfs-only update rewrites boot partitions.
- The 3.10 rootfs includes a LoadPin/verity trustpoint configuration
  service. Linux security policy is distinct from silicon BootROM
  signature enforcement.
- Fastboot boot completion invokes bootm and then CPU reset if
  bootm returns. Permission global 0x0014d690 has only three
  Ghidra references in the current P3 report; its runtime state
  has not been observed.

### New prioritized questions

| ID | Question | Next evidence source / closure criterion |
| --- | --- | --- |
| Q65 | Why do boot.scr.uimg hashes differ when commands match? | V4 decode U-Boot script multi-image table and compare normalized script hashes |
| Q66 | Which bytes/partitions are hashed by az01-bootloader-sig? | ARM ELF disassembly of actual referenced paths and read offsets |
| Q67 | Does a mismatched primary/fallback loader only warn, halt boot, or trigger recovery? | Trace post-hash branch and executable callers |
| Q68 | What role does eMMC partition 11 play in signing/recovery? | Trace bootloader-sig and az01config; compare physical partition map |
| Q69 | Did 3.6 already have az01-bootloader-sig and invoke it at startup? | V4 independent baseline-P6 vs 3.10 file/service inspection |
| Q70 | Does image signature verification run on every update? | libaz01-prog signature and image verification call graph, image format checks |
| Q71 | Can an official update change P1/P3 despite the current update being rootfs-only? | Trace write_bootloader/commit_bootloader call paths, partition record constraints |
| Q72 | Does the updater enforce rollback version or fuse policy? | Updater executable disassembly, signature checking and version checks |
| Q73 | How do LoadPin/verity trustpoints get initialized? | Inspect service executable and kernel config; distinguish post-boot policy |
| Q74 | Which 3.10 DTB properties *actually* changed? | Structural node/property diff of target acva and acva-c DTBs |
| Q75 | Are early U-Boot MMC aliases consistent with Linux SD/eMMC controllers? | Cross-stage DTB and board initialization; runtime enumeration later |
| Q76 | Is U-Boot embedded DTB specific to this MPC One PCB revision? | Trace board revision selection against DTB compatible and fdtfile |
| Q77 | Are 2016 U-Boot bootz and 3.10 kernel/FDT ABI compatible? | Compare boot script, image header, bootz init, kernel/DTB runtime success |
| Q78 | Can transient boot selection force mmc1 despite successful eMMC boot? | U-Boot console, environment, recovery ADC and boot order paths |
| Q79 | Can SD boot use standard boot.scr/bootz without modifying eMMC? | Prove nonpersistent selection and safe kernel/FDT RAM layout |
| Q80 | What does scriptaddr zero actually mean on this build? | Trace env script loader and U-Boot relocation; prove safe script buffer |
| Q81 | Does a failed Fastboot RAM boot always reset rather than resume USB? | Fastboot boot-done reset path; cautious authorized runtime test later |
| Q82 | Which of 810 previously undetailed P3 functions close outstanding boot/security gaps? | V4 recognized-function coverage pass, prioritized by callgraph centrality |
| Q83 | Where do SPL MMC callback pointers and U-Boot indirect handlers lead? | V4 computed-call instruction windows and typed vtable data |
| Q84 | Is there a credible software path to *genuine silicon* MaskROM? | Distinguish PMU loader flags from ROM entry, then true ROM enumeration |
| Q85 | Which stage reads secure vs ordinary eFuse registers? | Inspect ffB10000 vs ffB40000 static accesses; hardware read still required |
| Q86 | Can Linux verity/loadpin obstruct MPFree userspace or kernel boot? | Kernel config, systemd service and mount-path dependency tracing |
| Q87 | What is the maximum demonstrably non-overlapping RAM boot image? | Reconstruct relocation/BSS/stack/USB download, not arbitrary 1.75GiB limit |
| Q88 | Which remaining questions now require hardware observation? | Evidence ledger: ROM enumeration, USB wiring, fuse status, owner SPL acceptance |

### V4 run additions

MPFreeBootChainCoverageV4.py consumes each SHA-verified V3 graph and
decompiles **all previously undetailed Ghidra-recognized functions**:
127 P1 and 810 P3 in this owner's latest report. Indirect call sites
also include nearby raw ARM/Thumb instruction windows. This improves
analysis coverage, not proof that Ghidra recognized every function.

offline-bootchain-v4-compare.py copies factory P6 to a temporary
regular file, compares normalized 3.6/3.10 boot scripts and device
tree properties, and inspects selected Akai updater/security programs
via ARM ELF symbol tables and disassembly. A new 3.10 updater RAMFS
file listing is included if unsquashfs is available.

The existing runner packs all JSON/text/log results into ONE private
ZIP at exit. No firmware binaries, whole-disk backup, Ghidra projects,
or active bootloader execution are included.

Static evidence cannot confirm silicon MaskROM, unsigned SPL acceptance,
the existence of an independent eMMC restore path, or runtime Fastboot
authorization: those remain deliberately separate hardware proof gates.


## Fourth owner ZIP: 2026-11-11T132338Z_3414727 (Q89–Q110)

41 report/log files passed the owner ZIP_INDEX SHA256 checks.
All major V4 analyses and firmware comparisons completed.

### Verified findings and limitations

- Ghidra recognized 213 SPL and 1050 U-Boot functions; V3+V4
  processed all of their entries. Three P3 decompilations at
  00100080, 00100200, 00100260 FAILED (memory-range errors).
  There is also an ARM instruction-decoding warning in SPL.
  Processed is NOT the same thing as successfully decompiled.
- Both examined 3.10 MPC One board DTBs add one boot-relevant
  property relative to 3.6: mode-update = 0x5242c301. This
  aliases the existing mode-loader value 0x5242c301 in the
  PMU ff730094 reboot-mode node, not a new physical boot request.
- Executable boot-script contents match between 3.6 and 3.10
  after removing the legacy-image wrapper and trailing newline.
- Firmware 3.10 contains az01-bootloader-sig (9,616-byte ARM ELF)
  absent from the examined factory 3.6 rootfs. The machine code
  contains a 32-byte memcmp of calculated values, and a mismatch
  branches to errx. The program also uses SHA256 and RC4.
  This is a userspace integrity utility with a fatal error path
  WHEN THE CLI EXECUTES; invocation at boot and RC4's role
  are not established. It does NOT prove silicon secure boot.
- 3.10 also contains az0x-loadpin-trustpoints and an early
  oneshot LoadPin systemd unit; both absent from 3.6 factory
  files. The unit runs before local-fs-pre.target, but this
  is Linux/kernel policy, not a BootROM verification rule.
- az01-update and libaz01-prog.so.1.3 exist in both systems,
  but have different SHA256 hashes. Symbols remain similar;
  functional equivalence cannot be assumed.
- New 3.10 az01config contains open/read/crc32_z/write/fsync
  call sites; the file path and command mode must be traced.
  Do NOT execute it as part of offline reverse engineering.
- The 3.10 ramfs-update.squashfs was found and file listed,
  but its internal scripts remain to be inspected offline.
- Existing Fastboot boot_done has a bootm then reset path;
  permission value 0x0014d690 is runtime unknown.
- Both board DTBs differ in one SELECTED boot node, which does
  not prove the entire DTB differs only in that node.

### New questions (Q89–Q110)

| ID | Open question | Next offline evidence or hardware proof |
| --- | --- | --- |
| Q89 | Which Linux callers use update vs loader reboot mode? | Service/helper string xrefs and known register values |
| Q90 | Can update be distinguished from loader in the PMU? | No: same numeric value; investigate downstream context |
| Q91 | What is the bootloader-sig mismatch action? | V5 ELF call/control-flow decompilation |
| Q92 | What does RC4 do in bootloader-sig? | Trace RC4 key, input, output relationship to SHA digest |
| Q93 | Which partitions and exact ranges are hashed? | V5 ELF static file path and argument xrefs |
| Q94 | What is primary vs fallback bootloader in this utility? | Map partitions P1/P2/P11 against ELF source paths |
| Q95 | Is bootloader-sig run automatically at boot? | Search startup services and execution references |
| Q96 | What does LoadPin helper open and ioctl? | Imported ELF decompilation and service definition |
| Q97 | Are LoadPin failures fatal to a successful boot? | Kernel config plus init/systemd dependencies |
| Q98 | Can az01config update P2 environment? | Examine open/write/fsync paths, never execute CLI |
| Q99 | What code changed between 3.6 and 3.10 updater ELF? | Function/body and import relocation comparison |
| Q100 | Which signature checks are mandatory for update images? | Library caller graph and conditional branches |
| Q101 | Can rootfs-only 3.10 execute bootloader write paths? | Trace PART selection and code invocation separately |
| Q102 | Are three failed P3 Ghidra functions executable code? | Raw ARM/Thumb memory and entry-point validation |
| Q103 | Can valid-eMMC RK3288 enter genuine silicon MaskROM? | ROM reset mode and physical USB enumeration |
| Q104 | How can mmc1 be booted when mmc0 script succeeds? | Transient U-Boot input, UART, keys, SD detect |
| Q105 | Is scriptaddr zero a safe temporary boot-script buffer? | Source/sysboot and U-Boot relocation analysis |
| Q106 | Does Fastboot GETVAR require the same permission as download? | Dispatch permission/read-only graph |
| Q107 | What happens after a Fastboot RAM-boot failure? | Validate bootm reset path and PMU state |
| Q108 | Is there a proven independent eMMC restore route? | Genuine ROM readback/restore verification |
| Q109 | Does independent Linux bypass userspace LoadPin policy? | Separate ROM/SPL/U-Boot/kernel/userspace trust |
| Q110 | What cannot be resolved by another offline run? | Evidence ledger and safe, optional hardware probes |

### Next broad but focused V5 pass

1. offline-bootchain-v5-synthesis.py consumes all recognized V3/V4
   function call graphs/decompilation results: reconstructs direct
   shortest paths, groups global-state interactions, ranks indirect
   callbacks and labels unproved hardware assumptions.
2. offline-bootchain-v5-elf-import.py imports only the three newly
   significant 3.10 helpers (bootloader-sig, loadpin, az01config)
   into EPHEMERAL private Ghidra projects, automatically deleted.
   It emits function pseudocode and strings/xrefs into the single ZIP.
3. No firmware execution, live MPC access, flashing, environment
   edits, boot-partition writes, ROM requests or OTP programming.

This replaces endless undirected re-decompilation with testable
questions and explicit hardware-proof gates.


## Fifth owner ZIP (2026-11-11T192922Z-v5-3534429): Q111–Q148

The owner-supplied incremental V5 archive had **30 indexed files**, all
individually verified against the private ZIP's SHA-256 index. All three
temporary Ghidra ELF imports completed (no target binaries executed).

### Important new evidence and corrections

- **Ghidra V5 ELF coverage was shallow in the actual application logic.**
  Bootloader-signature: 39 recognized functions, most import stubs;
  LoadPin: 38 recognized, most stubs; az01config: 52 recognized, most
  stubs. Main logic for these stripped PIE executables was NOT fully
  identified. This is a known class of Ghidra ARM ELF GOT/relative
  relocation issue; do not describe this as 100% decompilation.
- The **az01config** ELF includes a literal **program-fuses** command
  (three Ghidra data references), along with eMMC path literals
  for partitions 3, 4 and 11 and write/fsync/CRC32 imports.
  Static evidence does not establish its arguments or precisely
  which hardware resources it may program.
  **Never execute az01config or program-fuses as a diagnostic.**
- The standalone az01-bootloader-sig ELF contains signed-image
  header diagnostics, eMMC partition 1/2/11 paths, RC4 and SHA256,
  a 32-byte digest comparison, and an errx fatal-failure branch.
  Whether the utility is invoked during ordinary boot remains
  unproven.
- LoadPin helper names /sys/kernel/security/loadpin/dm-verity,
  /usr/lib/az0x/trusted-verity-digests and /dev/null, and imports
  ioctl. This supports Linux userspace trustpoint configuration,
  not silicon ROM signature enforcement.
- V5 P1 graph: 213 recognized, 366 direct edges, 19 computed call
  sites. P3: 1050 recognized, 2840 direct edges, 167 computed call
  sites. Three P3 low-level decompilation failures remain.
- SPL boot loop -> media load and indirect handoff are in the direct
  call graph; Fastboot RAM completion -> bootm -> image loading is
  also a direct call chain. Other dispatch relationships remain
  unresolved by direct-call-only analysis; indirect calls/jump
  tables may bridge those paths.

### New question clusters

| IDs | Focus | What V6 looks for |
| --- | --- | --- |
| Q111–Q116 | True silicon ROM, USB OTG wiring, SD/eMMC boot priority, extCSD and independent eMMC recovery | Differentiate BootROM from Fastboot; document hardware-only proof gaps |
| Q117–Q121 | SPL entry, ARM/Thumb mode, media/DDR init, image checks, true second-stage branch | Backward/forward two-hop call graph and computed-call function ranking |
| Q122–Q126 | Direct SD boot through stock U-Boot without modifying eMMC | Resolve boot scan, interactive U-Boot controls and temporary environment |
| Q127–Q131 | Fastboot permission, getvar, RAM image type and memory overlap | Rank reachable Fastboot handlers, source/sink pseudocode and reset path |
| Q132–Q139 | az01config program-fuses, partition writes, SHA/RC4 bootloader hash, LoadPin ioctl, invocation at boot | ELF32 ARM R_ARM_RELATIVE .text pointers, missing stripped-PIE main decompilation and service/config references |
| Q140–Q144 | Firmware-upgrade resilience and long-term MPFree independence | Separate kexec-independent SD boot from true owner-controlled SPL and boot manager |
| Q145–Q148 | Recovery fault model, observational tests, independent hardware restore rehearsal | Explicit no-100%-guarantee statement and verified external proof gates |

The exact wording, evidence type and closure plan of every Q111–Q148
question is machine-readable in **v6-static-evidence.json**, emitted
by tools/mpc-one/offline-bootchain-v6-investigate.py.

### V6 incremental investigation

The recommended entry point is:

    python3 tools/mpc-one/run_bootchain_v6_followup.py \
      ~/mpfree-work/bootchain-wide-20261111T192922Z-v5-3534429.zip

It reuses and SHA-validates the prior ZIP instead of rerunning every
known P1/P3 decompilation, investigates the 186 recorded indirect call
sites, analyzes command/callsite evidence in the private ELF disassembly,
performs bounded read-only searching of the already extracted 3.10
ext4 systemd/config files and derives code-pointer candidates from ARM
PIE R_ARM_RELATIVE relocations. Where available, it attempts temporary
Ghidra function recovery for the missing stripped ELF main/callbacks.
All resulting artifacts are bundled in **ONE PRIVATE ZIP**, with raw
Akai firmware and executables excluded.

The complete proposed R01–R10 runtime protocol, recovery acceptance
criteria and fault-model disclaimers are in
[MPFree runtime and recovery research plan](mpfree-runtime-recovery-research-plan.md).

**Never mistake fallback SD boot, ordinary U-Boot Fastboot, or the
CONFIG_SPL_ROCKCHIP_BACK_TO_BROM normal handoff for a verified,
bootloader-independent silicon USB recovery channel. The existing
user-area eMMC backup may omit boot0, boot1, RPMB and hardware eFuse
configuration. Even a validated restore procedure cannot guarantee
recovery from physical failures or irreversible fuse programming.**


## Sixth owner ZIP (2026-11-11T223243Z-v6-3593975): Q149–Q180

The private V6 ZIP contained 49 SHA256-indexed reports/logs and passed
all index and ZIP integrity checks. Its offline static, ELF relocation,
startup-service scan and three temporary Ghidra ELF recovery stages
all completed successfully. No MPC was contacted.

### New conclusions and corrected limits

- The previously missing owner ELF application logic was at least
  partially recovered: bootloader-sig main candidate 0x76c,
  LoadPin main candidate 0x6c4, and az01config dispatch candidate 0x978.
  ELF addresses are module-relative candidate virtual addresses;
  they are NOT live kernel virtual addresses or native firmware
  entrypoints. Ghidra printed bad-instruction/undefined-GOT warnings.
- az01config compares a command string across **five 12-byte dispatch
  records**, then invokes a handler function pointer. This provides
  a static path to map commands, especially the potentially
  irreversible program-fuses command, without running that ELF.
  V7 locates the table using relocation-backed ELF32 pointers.
- Bootloader-sig compares two 32-byte buffers and reaches errx(1)
  on mismatch. This proves an error path when the CLI executes;
  it does not prove the CLI is invoked at ordinary startup.
  The bytes hashed and the RC4 transform still require tracing.
- LoadPin helper opens /sys/kernel/security/loadpin/dm-verity and
  issues ioctl 0x40044c00 with the trusted-digests file descriptor.
  This equals the Linux UAPI LOADPIN_IOC_SET_TRUSTED_VERITY_DIGESTS
  (_IOW('L',0,unsigned int)). This is Linux LSM policy,
  NOT proof that BootROM/SPL verifies MPFree images.
- Of 390 text/config files examined in the 3.10 startup scan,
  **zero** contained the bootloader-sig and program-fuses markers.
  The scan contains an early LoadPin systemd service and standard
  systemd kexec units. A text-only zero hit CANNOT rule out
  binary invocation, runtime strings, generated service files,
  or calls from other update environments.
- All **213** recognized SRAM SPL functions have pseudocode;
  **1044 of 1050** recognized U-Boot functions have pseudocode,
  with six Ghidra low-level decompilation failures. A few
  previous summary messages counted only three, but the V6
  report's current evidence is SIX failures.
- P1's 19 computed call sites and P3's 167 remain key unresolved
  indirect edges. A complete list of recognized functions
  does NOT imply a complete reachable boot execution graph.

### Q149–Q180 added, grouped by evidence needed

| Questions | Research focus | Next investigation |
| --- | --- | --- |
| Q149–Q152 | az01config command table; program-fuses targets; partition write/fuse safety | Relocation-backed name/handler mapping and callsite triage; do not execute |
| Q153–Q156 | bootloader-sig input ranges, RC4/SHA256 and invocation reachability | Recovered main, ELF unwind code pointers, service/binary references |
| Q157–Q160 | LoadPin ioctl, digest source, error fallbacks, kexec interaction | Cross-reference Linux UAPI and Ghidra main; kernel state still runtime |
| Q161–Q163 | Firmware 3.10 utilities used by other applications, OTA bootloader writes | Bounded offline ELF/text reference scan; later updater call graph |
| Q164–Q167 | Genuine MaskROM USB, independent eMMC restore, physical USB wiring | Hardware only, with independent non-destructive recovery gates |
| Q168–Q170 | SPL function callbacks, DDR and owner-built loader acceptance | Static edge recovery + future gated RAM-only hardware tests |
| Q171–Q174 | SD-first boot, vendor U-Boot dependence, official update resilience | U-Boot input/path/boot-script analysis, independent boot prototype |
| Q175–Q177 | Fastboot permission, safe RAM image size and failure/reset | Data-flow and measured runtime memory, no image execution yet |
| Q178–Q180 | Backup coverage, recoverability fault model and release criteria | Real storage inventory and sacrificial restore rehearsal |

Exact wording of all 32 questions and per-question evidence goals
will be emitted in the private v7-evidence.json from the next run.

### V7 entrypoint and evidence expectations

The V7 run reuses the **verified V6 ZIP** rather than repeating
1263 recognized bootloader decompilations and three already
recovered ELF main routines. It adds:

1. offline-bootchain-v7-elf-dispatch.py: parse ELF32/ARM relocation-backed
   az01config command tables; record candidate command-to-handler
   mappings without executing potentially destructive utilities.
2. offline-bootchain-v7-unwind.py: discover extra candidate
   stripped-function boundaries using ARM EHABI .ARM.exidx PREL31
   records, complementing the V6 ELF RELATIVE pointer pass.
3. offline-bootchain-v7-reference-scan.py: bounded binary/text literal
   search to find possible external invokers of bootloader-sig,
   program-fuses, LoadPin and kexec. This is NOT runtime proof.
4. offline-bootchain-v7-synthesis.py: join source-backed evidence,
   all 32 new questions, unresolved indirect calls and hardware
   recovery gates in a private human/JSON evidence summary.
5. run_bootchain_v7_followup.py: verify source ZIP index and stage/ELF
   hashes, run every optional offline analysis and produce ONE indexed
   private ZIP with logs even if optional stages fail.

This audit neither enters MaskROM nor modifies eMMC, Akai Linux,
firmware, environment, fuse or UART/USB device state.

### Honest recovery boundary

Until silicon ROM access and independent readback/restoration have
actually been demonstrated on this MPC, **none** of the following
can honestly be considered 100% recoverable: changing P1/P3,
modifying eMMC boot configuration, a failed direct RAM kernel boot,
or attempting an unsigned SPL. A verified user-area mmcblk0.img
backup is not proof of recoverability from a destroyed SPL. Testing
must preserve a separately verified restore mechanism and explicitly
exclude electrical failures, degraded flash and irreversible OTP.
