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
