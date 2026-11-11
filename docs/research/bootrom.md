# MPC One RK3288 BootROM, p1 ID block and reset-mode reverse engineering

**Status:** offline reverse engineering, 2026-11-10. **Device safety:** no eMMC writes, no new device-side operations or firmware changes were performed for the findings in this document. Only this file was authorized for creation/update.

## Scope, provenance and confidence

This note covers the reference **Akai MPC One** and its existing verified **3.6.0** eMMC backup; it is **not** a verified description of a 3.10.0 bootloader. Measurements below are from the owner's private offline analysis reports unless marked as external reference or inference.

- **Measured:** p1's extracted 64 KiB partition, the decoded first Rockchip header sector, the 32 KiB first-stage blob, p3's extracted U-Boot uImage and its Ghidra disassembly, and the owner's read-only SSH boot map.
- **Corroborated by external source:** Rockchip image-format fields, RK3288 SRAM load convention, ARM/Thumb and U-Boot reset API behavior.
- **Inferred or unknown:** p1's exact BootROM-return behavior, post-reset boot decision, true MaskROM entry on this board, secure-boot eFuse state, and whether the entire p1 stage operates like upstream SPL.

Existing project context: [boot map](../hardware/boot-map.md), [MaskROM/USB investigation](../hardware/maskrom-usb.md), [verified full eMMC backup](../hardware/emmc-backup.md). The raw, device-specific binary files and reports remain private on the owner's Linux workstation and MUST NOT be committed.

## 1. Boot chain and disk geometry

The RK3288 has an immutable silicon BootROM. On the reference unit, its eMMC first-stage image begins in the **user area** at LBA 64; eMMC boot0 and boot1 hardware regions were all zeros in the earlier verified survey.

| Image/location | eMMC location | Size | Role |
| --- | --- | ---: | --- |
| `mmcblk0p1` / `uboot-spl` | LBA 64, byte offset `0x8000` | 64 KiB | Rockchip ID-block header and initial stage |
| `mmcblk0p2` / `env` | LBA 192 | 8 KiB | U-Boot environment |
| `mmcblk0p3` / `uboot` | LBA 256 | 384 KiB | Legacy uImage containing U-Boot proper |
| `mmcblk0p6` / `rootfs` | LBA 17408 | 500 MiB | Linux kernel, DTBs, boot scripts, root filesystem |

The complete reference backup was made and independently verified on 2026-11-09. Its SHA256 hashes for p1 and p3 are:

- p1: `aa36e0e4033ce7386a210ad3e41f405a0d952a5b2a2c2711328450017414f1ec`
- p3: `7b6c8d091e7fcaecb89e633bacc1b541f4c494844b23daae6dc9a3c564721976`

The eMMC image is private; it may contain proprietary samples, projects, settings, machine identifiers and credentials. Neither the full image nor extracted Akai bootloader bytes belong in this repository.

## 2. p1 Rockchip legacy ID block: verified header

The owner ran `mpfree_p1_idblock_analyze.py` on the extracted reference p1; the source SHA256 matched the baseline. The first 512-byte sector is RC4-encoded with Rockchip's published key:

`7c4e0304550509072d2c7b38170d1711`

The first 16 raw bytes were `3b 8c dc fc be 9f 9d 51 eb 30 34 ce 24 51 1f 98`. Decrypting with the published key yields:

```text
55 aa f0 0f  00 00 00 00  01 00 00 00  04 00 00 00
```

| Decoded `header0` field | Offset | Measured | Interpretation |
| --- | ---: | ---: | --- |
| `magic` | `0x000` | `0x0FF0AA55` | Legacy Rockchip signature |
| `disable_rc4` | `0x008` | `1` | Initial payload is plaintext |
| `init_offset` | `0x00C` | 4 sectors | Initial stage starts `p1+0x800` |
| `init_size` | `0x1FA` | 64 sectors | 32,768-byte initial stage |
| `init_boot_size` | `0x1FC` | 1,088 sectors | 557,056-byte boot-chain size field; **not** the amount of executable inside 64-KiB p1 |

The format of these fields is defined in U-Boot `tools/rkcommon.c`; the `init_boot_size` field participates in Rockchip multi-stage/BootROM loading conventions. Its exact use on this unit remains to be traced. **Do not try to extract 557,056 bytes from the 65,536-byte p1 based on this field.** Reference: https://sources.debian.org/src/u-boot/2021.01%2Bdfsg-5/tools/rkcommon.c/

### Stage organization

```text
p1-relative       eMMC-image-relative     Description
0x0000            0x8000                  RC4-encoded header0
0x0200..0x07ff    0x8200..0x87ff          Header padding/reserved
0x0800            0x8800                  "RK32" (four-byte stage marker)
0x0804            0x8804                  Stage code-file begins; 28 initial zero bytes
0x0820            0x8820                  ARM reset vector (word 0xEA00000F)
0x0824..0x083c    0x8824..0x883c          ARM exception vector LDR instructions
0x0840..0x085b    0x8840..0x885b          Seven pointer words of 0xFF704060
0x0860            0x8860                  Initial ARM exception/trap code region
0x0864            0x8864                  Reset-vector branch target
0x8800            0x10000                 End of 32 KiB initial stage
```

The raw stage begins with `RK32`, 28 zero bytes, and the ARM vector sequence. The repeated pointer `0xFF704060` is strong additional evidence for the SRAM address mapping discussed below. The `0xEA00000F` reset-vector branch targets `0xFF704064` under the expected mapping.

The stage may contain code, literal pools, command tables, and an embedded DTB. It must **not** be treated as 32 KiB of contiguous ARM instructions.

### RK3288 SRAM mapping for Ghidra

Upstream U-Boot's RK3288 documentation describes loading an `RK32` image at **`0xFF704000` in internal SRAM**, with the four-byte `RK32` prefix; legacy `CONFIG_SPL_TEXT_BASE` is **`0xFF704004`**.

- `p1-initial-stage.bin` includes `RK32`; if imported as raw, its candidate base is `0xFF704000`.
- `p1-initial-stage-code.bin` **omits** only the four-byte `RK32` marker, **not** the 28 following zeros; import it as **Raw Binary**, **ARM:LE:32:v7**, compiler spec **default**, base **`0xFF704004`**.
- At that base, `p1+0x820` maps to **`0xFF704020`**, `p1+0x840` to `0xFF704040`, and `p1+0x860` to **`0xFF704060`**. This matches the observed embedded exception handler pointers.
- Seed ARM disassembly at `0xFF704020`; the reset vector branches to **`0xFF704064`**. Follow code flow rather than disassembling the intervening literal pool as instructions.
- Do not strip 32 bytes *and then* import at `0xFF704004`; that would shift every reference by 28 bytes.

External references:
- https://android.googlesource.com/platform/external/u-boot/+/d58779cb6425416074fce00f36066d76dc6d850b/doc/README.rockchip
- https://android.googlesource.com/platform/external/u-boot/+/f7aa2094d4b1f8121384a030e95177327d873339/include/configs/rk3288_common.h

This is a strongly corroborated **offline candidate mapping**; confirmation by disassembled control flow is the next step.

## 3. Boot-mode literals found in p1

The owner searched both the raw 64-KiB p1 and extracted plaintext initial stage. The results were:

| Constant/string | p1-relative location | Status |
| --- | --- | --- |
| `0xFF730094` (RK3288 reboot-mode register) | `+0x1A90` | **Found** |
| `0x5242C309` (Rockchip Fastboot request) | `+0x1A94` | **Found** |
| `0xEF08A53C` (BootROM download request) | — | **Not found** |
| `0x5242C301` (Rockusb/Loader request) | — | **Not found** |
| `reset_bootloader` | `+0x6267` | **Found** |
| `reset to fastboot mode` | `+0x6281` | **Found** |
| `rk3288_sysreset` | `+0x61E3` | **Found** |
| `rockusb`, `back_to_bootrom`, `maskrom` | — | Not found in the prior scans |

These findings show that the small initial-stage image itself contains a Fastboot-mode marker, a reset-mode register literal and associated reset-related strings. The exact code paths using these words have **not** yet been disassembled and established. A missing literal or name does not rule out synthesized constants, an alternate ROM-handoff sequence, a hidden later loader, or the behavior of on-chip ROM.

## 4. p3 U-Boot proper: Fastboot command confirmed

The extracted 3.6.0 p3 U-Boot legacy uImage was validated with both header and payload CRCs. Metadata:

| Attribute | Observed |
| --- | --- |
| Name | `U-Boot 2016.05-inmusic-20191010` |
| Architecture | ARM |
| Payload | 363,649 bytes, uncompressed |
| uImage load address | `0x00100000` |
| uImage entry point | `0x00000000` |
| Ghidra language | `ARM:LE:32:v7` |

Raw pointer recovery established that internal command/string pointers are linked against base **`0x00100000`**, even though initial Ghidra work mapped the raw file at zero. For subsequent cross-reference recovery, reimport the raw payload at `0x00100000` and preserve the previous Ghidra database.

### `reset_bootloader` command descriptor and handler

The command table at p3 raw file offset `0x44DF4` contains:

```text
name pointer       0x0013B670 -> "reset_bootloader"
maxargs            2
repeatable         1
handler pointer    0x00100E45 (Thumb pointer; instruction address 0x00100E44)
usage pointer      0x0013B681 -> "reset to fastboot mode"
```

Verified Thumb disassembly at raw payload offset `0xE44`:

```asm
push {r3,lr}
ldr  r3,[pc,...]    ; literal 0xFF730094
ldr  r2,[pc,...]    ; literal 0x5242C309
str  r2,[r3]        ; write Fastboot request to reboot-mode register
movs r3,#0
mov  r2,r3
mov  r1,r3
mov  r0,r3
bl   0x0011622A    ; reset-related wrapper
movs r0,#0
pop  {r3,pc}
```

The same `0xFF730094` / `0x5242C309` pair appears near raw offset `0xEDC` in a second flag-only routine.

**Confirmed:** `reset_bootloader` requests **Fastboot** at the register level. **Not confirmed:** any transition from this command to true MaskROM.

### Reset wrapper and halt fallback

The owner traced the function at raw offset `0x1622A` to `0x16216`, which calls `0x161D2`, compares its result with `-115` (`-EINPROGRESS`), waits with argument `100` when applicable, then calls a further routine at `0x2DB24`. This matches the expected pattern of U-Boot's system reset framework (`sysreset_walk_halt`/warm-reset and a nonreturning `hang` fallback), but the **exact function names and hardware driver dispatch are not yet conclusively identified**.

Decompiler output merged function-like code beginning at `0x16234` into the reset functions, likely because a nonreturning function was not marked as such. Use instruction-level control flow before trusting decompiled C. See original owner reports kept locally:

- `maskrom-ghidra-report.txt`
- `maskrom-ghidra-pointer-report.txt`
- `maskrom-uboot-reset-trace.txt`
- `maskrom-ghidra-follow-bl.txt`
- `maskrom-reset-cpu-trace.txt`

## 5. Fastboot vs Rockusb Loader vs true MaskROM

| State | Running code | Current evidence |
| --- | --- | --- |
| Akai Update Mode | InMusic U-Boot Fastboot in p3 | Verified by boot env and handler disassembly |
| Rockusb Loader | Loader protocol code in RAM | No proven entry path in this unit |
| MaskROM USB download | RK3288 silicon BootROM | No proven entry path while valid eMMC loader exists |

The stock environment contains `az01_recovercmd=...; fastboot 0` and a failure branch to `reset_bootloader`; the owner has not yet demonstrated a reboot directly into true MaskROM. The expected BootROM-request magic `0xEF08A53C` has not appeared in the p1 or p3 raw searches. This is **negative evidence against an obvious literal-based path**, not a definitive exclusion of every possible route.

The primary pending question is whether the stock p1 first-stage code recognizes a special restart state or can otherwise return control to the silicon ROM in USB-download mode. A second unknown is the RK3288 secure-boot eFuse state; no offline firmware dump establishes it.

The USB-B socket is a strong candidate for the BootROM OTG port, but the physical mapping and MaskROM enumeration have **not** been verified on this unit.

## 6. Reproducible offline Ghidra workflow

The owner has extracted `p1-initial-stage-code.bin` under a private `~/mpfree-work/p1-analysis-*/` directory. Open it in a **new** Ghidra program with:

```text
Format        Raw Binary
Language      ARM:LE:32:v7
Compiler      default
Base address  0xFF704004
Initial ARM vector to inspect  0xFF704020
First branch target             0xFF704064
```

The analysis helper distributed separately from the repository:

- Verifies the expected `0xEA00000F` vector, SRAM literal targets, input size and base.
- Seeds ARM (not Thumb) disassembly at the known vector and branch target.
- Searches the whole stage for BootROM, Rockusb, Fastboot and reboot-mode constants; also examines pointer references, strings and candidate command tables.
- Prints the relevant code and, if possible, decompiles the first branch target.
- Saves a human-reviewable report under `~/mpfree-work/` outside Git.

The helper **may change the Ghidra analysis database** (disassembly and function boundaries) but does not patch the binary on disk or touch the MPC.

Next technical checkpoints, in priority order:

1. Inspect disassembly from `0xFF704064` and establish stage entry and memory/DRAM initialization flow.
2. Find code that loads or writes the Fastboot literal at p1 `+0x1A94`, and identify how the startup reboot mode is interpreted.
3. Trace command metadata and reset-related strings in p1 to machine-code function pointers.
4. Determine whether `0xEF08A53C` could be synthesized or handled through some other BootROM-return mechanism.
5. Investigate second-stage loading from p3 and the meaning of the observed `init_boot_size` on this particular layout.
6. Compare the offline DTB's reboot-mode configuration to the register-write behavior.
7. Require explicit owner approval and a documented recovery/rollback plan before any device-side reboot-mode experiment; **do not erase p1/p3 or alter the eMMC bootloader as a discovery method**.

## 7. Safety gates

- **Offline now:** read private p1/p3 backups, inspect strings and disassembly, create local Ghidra project(s). No MPC commands.
- **Read-only on the device later, only if separately approved:** passive console observation, firmware inventory, ROM/boot-state evidence.
- **State changing:** reboot-mode register writes, reboot into a loader, MaskROM handoff attempts, USB loaders, eMMC restore. These require explicit consent and a validated recovery procedure.
- **Never as a shortcut:** zero p1, destroy the GPT, short unidentified motherboard pads, or assume Fastboot mode is equivalent to MaskROM recovery.

## Public technical references

- U-Boot Rockchip boot image and RK3288 SRAM loading: https://android.googlesource.com/platform/external/u-boot/+/d58779cb6425416074fce00f36066d76dc6d850b/doc/README.rockchip
- RK3288 `CONFIG_SPL_TEXT_BASE=0xff704004`: https://android.googlesource.com/platform/external/u-boot/+/f7aa2094d4b1f8121384a030e95177327d873339/include/configs/rk3288_common.h
- Legacy Rockchip image `header0_info`, `init_boot_size` rationale: https://sources.debian.org/src/u-boot/2021.01%2Bdfsg-5/tools/rkcommon.c/
- Ghidra ARM-state disassembly command: https://github.com/NationalSecurityAgency/ghidra/blob/master/Ghidra/Features/Base/src/main/java/ghidra/app/cmd/disassemble/ArmDisassembleCommand.java
- U-Boot source code and architecture details also discussed in [MaskROM USB research](../hardware/maskrom-usb.md).

## 8. Follow-up: first Ghidra audit of p1 SPL (owner, 2026-11-10)

The owner imported `p1-initial-stage-code.bin` as `ARM:LE:32:v7` at initialized memory block `0xFF704004` and ran the corrected `MPFreeP1GhidraAnalyze` helper. The private report is `p1-ghidra-analysis-20261110T131114.txt`. **The report did not involve MPC hardware and did not change firmware bytes**, but the analysis script created some Ghidra database disassembly/function metadata.

### Verified first-stage identity and control-flow entry

- Identifiable build string: `U-Boot SPL 2016.05-inmusic-20191010 (Dec 17 2019 - 16:45:18)`.
- The ARM reset vector at `0xFF704020` is `0xEA00000F` and branches to `0xFF704064`; that stub then branches to `0xFF7040B4`.
- Seven exception vector pointer words at `0xFF704040..0xFF704058` contain `0xFF704060`.
- The analysis recognized 6,317 instructions, a number of SPL/MMC functions and source-level diagnostic strings. The initial decompilation is **not** a verified complete boot flow; initial stubs and nonreturning functions can confuse Ghidra's function reconstruction.

### Newly established literal references

| Object | SRAM location | Ghidra read reference | Confidence |
| --- | --- | --- | --- |
| RK3288 reboot-mode register literal `0xFF730094` | `0xFF705290` (p1 `+0x1A90`) | `0xFF70527A` | Measured reference |
| Fastboot magic `0x5242C309` | `0xFF705294` (p1 `+0x1A94`) | `0xFF70527C` | Measured reference |

The early SPL therefore contains **code referencing both literals**, not just unused string data. This does not yet prove the register-store instruction, handler invocation path, or any BootROM download behavior. Trace the instructions around `0xFF70527A`/`0xFF70527C` before assigning semantics.

### New command-descriptor lead

- SPL string `reset_bootloader`: `0xFF709A67`, with a raw pointer at `0xFF709D00`.
- SPL help text `reset to fastboot mode`: `0xFF709A78`, pointer at `0xFF709D10`.
- If the standard 32-bit U-Boot command table applies, the adjacent pointer at **`0xFF709D0C`** should be the handler. **Its actual value has not yet been read.** The descriptor's presence alone does not establish that an interactive command parser is reachable in SPL.

Additional SPL text references include `Trying to boot from`, `SPL: failed to boot from all boot devices` and `spl: mmc: wrong boot mode`, associated with the vicinity of `0xFF705350` and `0xFF7054A4`.

### Negative evidence for software MaskROM

The full 32,764-byte extracted SPL contains **zero exact 32-bit little-endian matches** for `BOOT_BROM_DOWNLOAD=0xEF08A53C`, `ROCKUSB_LOADER=0x5242C301`, or the alternate early BootROM tag `0x5242C302`, and zero aligned halfword hits for `0xA53C`/`0xEF08`. The Fastboot request `0x5242C309` was found once.

A software USB MaskROM path requires cooperation from early stage code. Mainline U-Boot's `bootrom.c` explains that the silicon ROM does **not** directly check the warm-reboot mode register: early loader code must check `BOOT_BROM_DOWNLOAD` and return to the ROM with the appropriate status. This behavior has **not** been shown in this 2019 InMusic SPL. See https://coral.googlesource.com/u-boot-mtk/+/refs/heads/master/arch/arm/mach-rockchip/bootrom.c .

**Next offline pass:** read the command descriptor's fields and handler pointer at `0xFF709D00..`, extract assembly around `0xFF70527A` and its function, decompile available functions without assuming a correct noreturn graph, and determine whether early startup checks the reboot-mode register. Separate report outside Git.

**No device-side register writes, reboots or eMMC modifications are authorized by this follow-up.**

## 9. SPL command-handler disassembly confirms the Fastboot register write (2026-11-10)

The owner's next private report, `p1-reset-mode-deep-dive-20261110T131705.txt`, verified the first-stage handler and its call target. This section **supersedes the earlier tentative interpretation in §8** that only code references to the Fastboot literals had been found.

### Recovered five-word command descriptor

| SRAM address | Value | Verified interpretation |
| --- | --- | --- |
| `0xFF709D00` | `0xFF709A67` | `"reset_bootloader"` |
| `0xFF709D04` | `2` | Maximum argument count |
| `0xFF709D08` | `1` | Repeatability flag |
| `0xFF709D0C` | `0xFF705279` | Thumb code pointer, actual instruction at `0xFF705278` |
| `0xFF709D10` | `0xFF709A78` | `"reset to fastboot mode"` |

**Correction to the report generator:** `0xFF709D14` points to `"fixed_rate_clock"`, unrelated to the command, and `0xFF709D18=0xD`. There is no basis to treat these as `help` and `completion` fields; the command descriptor likely ends after five 32-bit fields. Inspect adjacent data structures separately.

The command table's presence **does not mean** that the SPL provides a usable interactive command shell; the registration could be linked in but never reached during normal SPL execution.

### Thumb machine-code proof of the flag write

```asm
ff705278 push {r3,lr}
ff70527a ldr  r3,[0xff705290]  ; [0xff705290] == 0xff730094
ff70527c ldr  r2,[0xff705294]  ; [0xff705294] == 0x5242c309
ff70527e str  r2,[r3,#0x0]     ; write Fastboot flag to RK3288 mode register
ff705280 movs r3,#0
ff705282 mov  r2,r3
ff705284 mov  r1,r3
ff705286 mov  r0,r3
ff705288 bl   0xff708296       ; next reset-related function to inspect
ff70528c movs r0,#0
ff70528e pop  {r3,pc}
```

This proves that the *p1 SPL* handler, like p3's separate handler, contains a hardware register write equivalent to:

```c
*(volatile uint32_t *)0xFF730094 = 0x5242C309;
```

The branch target `0xFF708296` has **not yet been decompiled** in the supplied report, so its detailed behavior remains unknown. Do not label it definitively as a specific `reset_cpu` implementation until inspected.

### Further control-flow findings

- `0xFF7040B4` enters low-level CPU/MMU initialization through a branch to `0xFF704068`.
- `0xFF705208` sets a stack pointer from an embedded constant, performs initialization calls, clears a region of memory and branches indirectly to a next-stage function pointer; symbolic identities remain unverified.
- `0xFF705350` is a boot-device iteration routine, including MMC-load attempts and the printed `"Trying to boot from "` / `"SPL: failed to boot from all boot devices"` messages.
- `0xFF7054A4` contains MMC raw-image loading logic and indirect calls to the block-read callback; it is not a verified BootROM entry path.
- `0xEF08A53C` (BootROM request) and `0x5242C301` (Rockusb Loader request) remain absent as exact raw 32-bit constants in the extracted p1 code.

The current assessment is therefore **strong negative evidence against a simple literal-based software MaskROM route in the 2019 InMusic SPL**, but *not* proof that no other return-to-ROM mechanism exists. Distinguish register-written Fastboot from true MaskROM USB download.

### Next focused Ghidra work

Analyze the direct Thumb call target at `0xFF708296` (call from `0xFF705288`), its immediate callees, the `rk3288_sysreset` driver registration area, and any startup reads of `0xFF730094`. Use a bounded disassembly scope and preserve the distinction between Ghidra database changes and **no changes to executable bytes, device storage, or MPFree source files**.

The independently distributed `MPFreeP1TraceReset.py` script performs this follow-up locally in Ghidra. It does **not** belong in this repository under the current one-file authorization.

## 10. p1 follow-up: reset wrapper and driver-dispatch investigation (owner, 2026-11-10)

The owner supplied the private report `p1-reset-followup-20261110T132140.txt`. This is the latest **measured** p1/SPL result; previous interpretations of its reset call path should be evaluated against this report.

### Observed machine-code control flow

At `0xFF705278`, the already-established `reset_bootloader` Thumb function loads:

- `[0xFF705290] = 0xFF730094` (RK3288 boot/reboot-mode register).
- `[0xFF705294] = 0x5242C309` (Rockchip Fastboot reboot request).

It executes `str r2,[r3,#0]`, then zeroes r0..r3 and branches to `0xFF708296`.

At `0xFF708296`, the report shows:

```asm
ff708296 push {r3,lr}
ff708298 movs r0,#0
ff70829a bl   0xff708282
ff70829e movs r0,r0    ; may be padding / unreachable if reset never returns
ff7082a0 ldr  r3,[0xff7082b8]   ; Ghidra recognizes a different function here
```

**Direct conclusion:** the p1 command invokes `0xFF708282` with argument 0 immediately after writing the Fastboot flag. The actual instructions at **`0xFF708282` were not included in this report**. The combination of an apparently nonreturning call and code from a separate function starting `0xFF7082A0` can mislead Ghidra's decompiler; the generated C for `FUN_ff708296` incorrectly includes a pointer-table walk from the following function. Do not treat that decompiler output as established source logic.

The report also identifies an indirect function-pointer dispatcher near `0xFF7082C0`: it calls `0xFF7082A0` to find a registration, then loads a callback pointer at offset `0x14` and executes `bx r3`. Whether this dispatch specifically participates in reset handling **has not been proven**.

### Registered reset driver evidence

The SPL contains `rk3288_sysreset`, and the raw data word at `0xFF709F78` points to that name (`0xFF7099D9`); the next word is `0x38`. This is consistent with a U-Boot driver registration, but the report does not reconstruct a valid full driver structure or callback function. Do not conflate the mere presence of that driver with evidence of its specific runtime register writes.

The SPL startup and boot-device selection code is also present, with linked functions near `0xFF705208` and `0xFF705350`, but **no demonstrated recognition of `BOOT_BROM_DOWNLOAD=0xEF08A53C`**.

### Engineering interpretation

This follow-up strengthens the finding that the stock 2019 SPL implements a **Fastboot** reset path, but **does not prove a software route into the RK3288 silicon BootROM**. Exact `0xEF08A53C` and `0x5242C301` literals remain absent from the extracted initial-stage code; this is negative evidence, not an impossibility proof.

The reset framework behavior closely resembles U-Boot `reset_cpu()`, `sysreset_walk_halt()` and driver-model dispatch. However the exact identity of `0xFF708282` and the final RK3288 reset register accesses must be confirmed from its instructions. For context, upstream U-Boot's `drivers/sysreset/sysreset-uclass.c` implements `sysreset_walk_halt` and `reset_cpu`: https://android.googlesource.com/platform/external/u-boot/+/825936815a752ce15f5a883c23bc54be76403268/drivers/sysreset/sysreset-uclass.c

**Next offline step:** run the separately distributed **`MPFreeP1SysresetDeepTrace.py`** Ghidra script, which reads `0xFF708282`, disassembles already-recognized functions, checks raw callback pointers and emits a private report at `~/mpfree-work/p1-sysreset-deep-trace-*.txt`. It does not patch Ghidra, firmware or MPC storage. Any hardware reboot/BootROM experiment still requires explicit owner approval.

## 11. SPL sysreset call chain now identified (owner report, 2026-11-10)

The owner ran the separately distributed read-only Ghidra helper and provided the private report `p1-sysreset-deep-trace-20261110T132622.txt`. The active binary is the 2019 InMusic `p1-initial-stage-code.bin` with actual SRAM memory-block base `0xFF704004`. The report confirms the `reset_bootloader` register write and resolves the immediate call chain to the system-reset framework.

### Measured Thumb call sequence

```text
0xFF705278  reset_bootloader command handler
    LDR r3, =0xFF730094
    LDR r2, =0x5242C309
    STR r2, [r3]                  ; Fastboot requested, not MaskROM
    BL  0xFF708296
        0xFF708296  reset_cpu-like wrapper
            MOVS r0, #0           ; warm reset type
            BL   0xFF708282
                0xFF708282  sysreset_walk_halt-like routine
                    BL   0xFF70823E
                    ADDS r0, #0x73
                    BNE  fallback
                    MOVS r0, #0x64
                    BL   0xFF708DB0         ; likely mdelay(100)
                    fallback:
                    BL   0xFF708B74         ; likely nonreturning hang()
```

The `ADDS r0,#0x73` followed by `BNE` tests whether the reset-walk return is `-115`, i.e. `-EINPROGRESS`. This implementation closely matches upstream U-Boot `sysreset_walk_halt()` and `reset_cpu()`, whose contract is to initiate a reset, wait briefly if `-EINPROGRESS`, then halt if control returns. Comparison source: https://android.googlesource.com/platform/external/u-boot/+/825936815a752ce15f5a883c23bc54be76403268/drivers/sysreset/sysreset-uclass.c .

**Current confidence:** `0xFF708296` = reset_cpu-like wrapper and `0xFF708282` = sysreset_walk_halt-like routine are very high confidence from instruction structure. The exact U-Boot function at **`0xFF70823E`** has **not yet been disassembled in the supplied report**. Identifying the precise `rk3288_sysreset` request callback and hardware reset registers remains the immediate task. The potential `mdelay` and `hang` functions at `0xFF708DB0` and `0xFF708B74` likewise remain to be independently inspected.

### Important callback correction

The earlier indirect dispatcher at `0xFF7082C0` is called at least from **MMC image-loading code** at `0xFF705538`, and from `0xFF7071DA`. It looks up a registration and dispatches through a function pointer at offset `0x14`; **the current evidence does not show that it belongs to the sysreset call chain**. Do not call it the RK3288 reset driver callback without an actual call edge.

The string `rk3288_sysreset` is present at `0xFF7099D9`, with raw references at `0xFF706794` and `0xFF709F78`. They are candidates for reverse engineering the real driver, but no callback pointer has yet been recovered from the registered structure.

### Consequences for MaskROM

The available p1 and p3 disassemblies firmly establish **Fastboot** flag writes. They do not demonstrate a check for `BOOT_BROM_DOWNLOAD=0xEF08A53C` at first-stage startup, or any handoff to the immutable RK3288 BootROM. The absence of the magic word in scanned p1/p3 bytes is meaningful negative evidence but does not exclude computed registers or alternative ROM entry methods.

Next: collect the private Ghidra report from the separately distributed `MPFreeP1ResetDriverTrace.py` script. It inspects `0xFF70823E`, `0xFF708DB0`, `0xFF708B74`, the reset-driver name references, and bounded direct-call targets. It performs **read-only Ghidra database operations** and must not be added to the MPFree repository under the current one-file permission. No MPC reset or register writes are part of this investigation.

## 12. Whole-boot-chain research programme: BootROM, SPL, U-Boot, Fastboot, DTB and installer (2026-11-10)

### Scope of this round and new source distinction

The owner asked to expand the investigation beyond reset handling, to **early boot-mode decisions, Fastboot command behavior, complete architecture, and true MaskROM/BootROM code**. The latest owner report is `p1-reset-driver-trace-20261110T133440.txt`. Findings about this unit below derive from that report and earlier verified owner surveys; general RK3288/U-Boot behavior is labelled **upstream** and is **not assumed to be present** in the 2019 InMusic binary.

**Important for versions:** the existing [Akai updater offline research](../hardware/akai-updater-offline.md) established that the official `MPC-3.10.0-Gen1-update.img` is an **AZ01 container with a single xz-compressed rootfs partition**; it **does not contain replacement p1/p3 bootloader payloads**. Thus the reference unit's verified 2019 p1/p3 loaders remain the ones to map for the 3.6.0-to-3.10.0 RAM-only kexec tests. The rootfs/kernel/DTB and software-update logic do change. This statement is about the specific examined 3.10.0 image, **not** a guarantee that all future Akai update images leave the bootloader untouched.

### 12.1 Direct result of latest SPL reset-driver trace

The p1 SPL reset chain can now be reconstructed with high confidence from Thumb instructions:

```text
reset_bootloader                     ff705278
  store 0x5242c309 -> 0xff730094    ff70527e
  reset_cpu-like wrapper            ff708296
    sysreset_walk_halt-like         ff708282
      sysreset_walk-like            ff70823e
         attempts reset types 0..2
         discovers reset devices    ff7060b4
         requests driver reset      ff70822e
         advances device iterator   ff7060d2
      if result == -EINPROGRESS, delay(100)  ff708db0
      always call hang fallback             ff708b74
```

- `0xFF70823E` enumerates reset types, requests a reset device using device-model calls and returns `-115` when a reset is in progress. This precisely matches the control structure expected of U-Boot's `sysreset_walk()`; the symbolic function name is inferred, not from surviving symbols.
- `0xFF708DB0` calls `0xFF708D90(1000)` repeatedly for each millisecond, confirming that a `100` argument produces ~100 ms delay.
- `0xFF708B74` prints a diagnostic and branches to itself indefinitely, confirming it is a non-returning hang/error routine. Ghidra's decompiler incorrectly includes subsequent function bodies after it; prefer bounded assembly.
- Device registration string `rk3288_sysreset` is present at `0xFF7099D9` and referenced at `0xFF706794` and `0xFF709F78`.
- An indirect call at `0xFF7067DC` loads a callback from a device's driver operations table `[[dev]+0x3C]+0x34` and executes `BLX r3`. It **may be relevant to sysreset**, but the specific call edge to this function and its ultimate hardware register write are **not proven yet**.

Upstream U-Boot's Rockchip sysreset driver specifies an implementation such as `writel(0xECA8, CRU_BASE+0x1B4)` for a warm reset, or `writel(0xFDB9, CRU_BASE+0x1B0)` for cold reset. Using the RK3288's CRU base `0xFF760000`, these candidate hardware addresses are **`0xFF7601B4` and `0xFF7601B0`**. The actual InMusic SPL driver may compute these from a base pointer and offset, leaving no full 32-bit address literal; direct tracing is still needed. References:
- https://nest-open-source.googlesource.com/manifest_repos/u-boot/+/06419a4bae2adcdd0ed4e73f93f3f40a82172e45/drivers/sysreset/sysreset_rockchip.c
- https://nest-open-source.googlesource.com/manifest_repos/u-boot/+/06419a4bae2adcdd0ed4e73f93f3f40a82172e45/drivers/clk/rockchip/clk_rk3288.c
- https://docs.u-boot.org/en/stable/api/sysreset.html

### 12.2 Critical distinction: BootROM code is not stored in p1/p3

RK3288 **BootROM is immutable silicon ROM**, while the stock `RK32` loader in p1 is **InMusic U-Boot SPL** running from internal SRAM; p3 is U-Boot proper. The eMMC backup cannot contain the silicon BootROM program. Therefore:

1. **Static inspection of private p1/p3** can detect *calls back into the ROM*, saved ROM return context, boot-mode registers, Fastboot commands and USB code, but not the full silicon-ROM implementation.
2. **Public U-Boot source and Rockchip TRM** can establish ROM-entry/exit conventions, boot priority, SRAM map, register behavior and USB protocol design for relevant chips. They are not substitutes for physical measurements on the MPC One.
3. **Actual BootROM behavior on this unit** would need to be inferred from USB enumeration, protocol captures in a genuinely established MaskROM session, passive boot console evidence, or explicitly authorized JTAG/debug access where permitted by silicon security settings. Normal eMMC dump and Ghidra disassembly cannot prove the internal silicon-ROM implementation.
4. Code associated with **Rockusb Loader** may live in a downloaded RAM image/usbplug, not necessarily in p1 or p3; **Akai Update Mode** runs U-Boot Fastboot, not the silicon ROM USB protocol.

### 12.3 Two distinct BootROM handoffs, and the most important new target

Rockchip's U-Boot documentation says the RK3288 supports a **return-to-BootROM next-stage loading mode**: the silicon ROM loads SPL, SPL performs initialization, then returns to the ROM, which may load another stage. Upstream `SPL_ROCKCHIP_BACK_TO_BROM` is an optional configuration, **not yet established for this particular InMusic SPL**.

An additional **return-to-ROM download mode** is illustrated in later upstream `bootrom.c`: very early `save_boot_params()` saves a ROM-return context, optionally checks the scratch register for `BOOT_BROM_DOWNLOAD=0xEF08A53C`, clears it, and returns to ROM with a nonzero return value; the ROM itself does not inspect that scratch register in the described implementation. The ROM can also be asked to continue normal second-stage loading. The latter capability does not imply the former was enabled in InMusic's older code.

References:
- https://android.googlesource.com/platform/external/u-boot/+/d58779cb6425416074fce00f36066d76dc6d850b/doc/README.rockchip
- https://coral.googlesource.com/u-boot-mtk/+/refs/heads/master/arch/arm/mach-rockchip/bootrom.c
- https://qemu.googlesource.com/u-boot/+/fd6e425be243dce518a02710482514faccf3c211/arch/arm/include/asm/arch-rockchip/bootrom.h

**Priority:** trace ARM startup at `0xFF704020 -> 0xFF704064 -> 0xFF7040B4 -> 0xFF704068`, the ARM/Thumb handoff near `0xFF7041E4` and main initialization `0xFF705208`, looking for saved LR/SP, setjmp/longjmp, early return to ROM, and how boot-device source is passed to `0xFF705350`. Determine whether p3 gets loaded by SPL's own MMC loader or by the BootROM continuation code, or whether the implementation supports both. The existing `0xFF7054A4` MMC loader and its indirect block-read callback are evidence for SPL-side loading but do not yet prove all stage transitions.

### 12.4 Systematic MaskROM signature search — limits

For each of p1, p3, the extracted DTBs, the Linux reboot-mode configuration and any verified loader RAM blobs:

1. Search raw LE32 values for `0xEF08A53C` (BootROM download), `0x5242C301` (Rockusb loader), `0x5242C309` (Fastboot), and `0xFF730094` (reboot register).
2. Examine **references to each literal pool** and inspect the actual ARM/Thumb `LDR` / `STR` instructions to classify **read**, **write**, **compare**, or unused data. Confirm no false positives in ASCII, compressed data or data structures.
3. Search code for `MOVW`/`MOVT`, shifted `ORR`, or arithmetic that **constructs constants without storing them literally**; enumerate indirect branches and saved ROM return context.
4. Detect `RK32`, `RK33`, legacy `uImage`, FIT/FDT (`D00DFEED`) and embedded device trees, then reconstruct the second-stage loader's addresses and bounds from validated metadata, not just string matches.
5. Examine *both* reset writers and **early-stage readers** of `0xFF730094`. A reboot flag written by Linux/U-Boot cannot trigger ROM mode unless early code checks it.
6. Search strings and code references to `save_boot_params`, `back_to_bootrom`, `usbplug`, `rockusb`, `Fastboot`, `rk3288_sysreset`, `secureboot`, `getvar`, `oem`, `lock` and `unlock`. Strings are clues; linkage and control flow are required for conclusions.

The reference 2019 p1 and p3 lack exact `0xEF08A53C` and `0x5242C301` literals, so a stock **software-only** MaskROM request currently has **no positive evidence**. Absence cannot rule out synthesized constants, an unused continuation capability, or other stages. Do not modify the register and reset until a complete supported handoff has been established and the owner explicitly approves the device-side experiment.

### 12.5 Fastboot questions for full architectural coverage

The U-Boot environment says `az01_recovercmd=...; fastboot 0` and boot target order `mmc0 mmc1`. The full bootloader analysis must document:

- The exact **entry path**: update button/key sampling, `dorecovery` environment variable, Linux `reboot loader` / `reboot bootloader` scratch value, fastboot-request reset path, and the recovery condition on cold boot vs warm reboot.
- The actual **USB gadget controller**, physical USB-B path, descriptors (VID/PID/interface), Fastboot enumeration and handshake.
- The full **Fastboot command dispatcher** and capabilities: `getvar`, `download`, `boot`, `reboot`, `reboot-bootloader`, `flash`, `erase`, `format`, and vendor `oem` commands. Each must be classified by its code path, requirements and side effects before use.
- **Authorization / lock model:** whether Fastboot RAM boot is implemented and permitted, which commands require vendor unlock, partition allowlist/denylist, anti-rollback checks and signature policy. No assumption that the MPC Live's unlock state is identical to the MPC One.
- **Partition map:** relationship of U-Boot GPT template, the actual GPT, `p2` environment, `p6` kernel+DTB, `p7` content and `p8` data; CRC/signature validation and backup/restore rules.
- **Fallback behaviors:** corrupt kernel/rootfs, failed boot script, failed eMMC, removable SD, missing SPL and interrupted flash. The precise recovery tier depends on which stage can still execute.
- **3.6.0 vs 3.10.0 device trees:** `syscon-reboot-mode` properties (`mode-bootloader`, `mode-loader`, others), USB OTG/peripheral configuration, button nodes, eFuse/security nodes and memory reservations. A Linux DTB value is not proof the 2019 SPL implements an equivalent mode.

Offline mapping first. The appropriate *later*, owner-approved dynamic test is a **read-only** USB Fastboot enumeration/`getvar` capture in Akai Update Mode. Never send `flash`, `erase`, `oem unlock` or destructive commands as an exploratory test. True MaskROM `rkdeveloptool ld` and RAM-loader tests remain a separate, gated recovery investigation.

### 12.6 Where else to look

| Source | What we can establish | Status / constraints |
| --- | --- | --- |
| p1 SPL executable & embedded DTB | Early startup, boot-media choice, BootROM-return clues, reboot flag reads, reset driver | Private offline copy available; BootROM itself is not inside |
| p2 U-Boot environment | Exact boot/recovery scripts and target order; per-unit secrets possible | Prior boot map exists; redact serial/MAC and private parameters |
| p3 U-Boot proper | Fastboot dispatcher, USB gadget, commands, partition operations, key checks, kernel/DTB boot | Private offline copy available |
| p6 `/boot`, 3.6.0 and 3.10.0 | kernel command line, reboot-mode DTB and USB/efuse/buttons; boot script | Existing offline rootfs/DTB tooling in repository |
| 3.10.0 `AZ01` updater, its rootfs and RAM updater | Updated Linux/userspace interface, signing and update plan; **not** a new p1/p3 for this examined file | Current image has a rootfs-only `PART` record |
| Public RK3288 TRM, U-Boot source, Rockchip rkbin | ROM boot priority, SRAM conventions, CRU reset regs, return ABI, USB download and compatible DDR loaders | Upstream reference; may differ from board firmware |
| Off-device USB trace (future) | Observed Akai Fastboot protocol or real MaskROM enumeration | State-changing mode transition requires owner go-ahead; no flash |
| Passive UART/JTAG (future) | Actual boot source/ROM/SPL messages, optional debug trace and secure-boot state | Need board access, safety review, no assumption ROM is readable |

### 12.7 Installer implications and gates

The intended **USB-A stick installer** and **USB-B Fastboot/MaskROM recovery cable** are different product paths.

- **Track A, lowest risk:** USB autorun / persistent SSH on stock boot stack; install MPFree userspace to removable media or approved `/data` locations; keep `p1`–`p6` unmodified. Removal and power-cycle restore stock behavior.
- **Track B, RAM-only kernel testing:** already-demonstrated kexec to a USB-backed 3.10.0 kernel with eMMC controllers disabled, and power-cycle back into 3.6.0. This is our concrete non-destructive OS testing precedent. Extend it toward an MPFree distribution once userspace, display and safety gates pass.
- **Tier 2 repair:** Akai U-Boot Fastboot, conditional on intact p1/p3. It is a **rootfs/data recovery mechanism**, not a guaranteed repair for a destroyed first-stage loader.
- **Tier 3 repair:** independently proven silicon BootROM USB recovery, compatible DDR/usbplug loader, read-back of the entire eMMC matched against the SSH backup, then a separately authorized **restore** plan for p1/p3. No bootloader/env/critical-storage writes before Tier 3.

No need to add a persistent bootloader change as an early installer prerequisite. Map Fastboot and ROM fully, use stock userspace and RAM-only kexec first, and preserve the verified recovery path for the eventual custom OS.

### 12.8 Scripts distributed outside the repository

The independently delivered **`mpfree_bootchain_offline.py`** performs a privacy-conscious, read-only scan of p1/p2/p3 from the existing workstation eMMC backup, RC4 legacy header checks, uImage CRC checks, magic/string/FDT searches, and optional extracted DTB inspection or streaming scan of a supplied update image. It does not execute code or connect to the MPC.

**`MPFreeBootChainGhidraAudit.py`** runs in the Ghidra Python Console for **both** the p1 program (`ARM:LE:32:v7`, memory start `0xFF704004`) and the p3 raw payload program (base 0 or recommended `0x00100000`). It produces separate private stage reports with entry paths, code windows, literal cross-references, candidates for ROM return, indirect branch clues, Fastboot strings and potential embedded FDTs. It does not modify Ghidra's analysis database, firmware bytes or MPFree repository files. Neither script has been executed in the owner's Ghidra; Python syntax and the offline script's synthetic-image test have been performed.

This section and the accompanying earlier research chapters do **not** establish software MaskROM entry. The next evidence should be read-only reports from both stages and extracted DTB comparisons.

## 13. Complete p1/p3 boot-chain audits: confirmed CRU reset, SPL loader, Fastboot command leads and DTBs (2026-11-10)

**Owner data:** `bootchain-offline-20261110T134821.txt`, `bootchain-offline-20261110T134836.txt`, `bootchain-ghidra-p1-spl-20261110T134948.txt`, `bootchain-ghidra-p3-uboot-20261110T135049.txt`. All reports were generated offline or by Ghidra. This section **supersedes the earlier assertion that the RK3288 hardware reset callback remained unidentified.** It does not authorize hardware changes.

### 13.1 Firmware identity and container integrity

- Verified p1 and p3 SHA256 hashes still match the baseline; Rockchip p1 ID-block signature `0x0FF0AA55`, 32 KiB `RK32` initial stage, `disable_rc4=1`, first ARM reset vector `0xEA00000F` at p1+0x820.
- p3 U-Boot legacy uImage name `U-Boot 2016.05-inmusic-20191010`, 363,649-byte payload, linked/load address `0x00100000`, **both header and data CRCs pass**. p2 8-KiB environment's CRC also passes; all environment values were deliberately omitted from the shared report.
- p1 contains an in-bounds embedded DTB at **p1+0x6A7C** (3,239 bytes). p3 contains an in-bounds DTB at payload **+0x502F4** (`0x898D` bytes); an additional `D00DFEED` at payload `+0x287AC` has an **out-of-bounds claimed size** and must **not** be treated as a valid DTB without separate verification.
- A raw scan of the 3.10.0 `AZ01` update image does not inspect its compressed rootfs. The existing project extractor already established that this particular 3.10.0 update has only the rootfs payload and no replacement p1/p3 U-Boot.

### 13.2 A hardware-reset driver has been located in the p1 executable

**New measured function:** `0xFF704214` contains the actual reset operation in the 2019 InMusic SPL. Its Ghidra disassembly (Thumb) is:

```asm
ff704214 push {r4,lr}
ff704216 mov  r4,r1             ; reset type from callback
ff704218 bl   0xff706798       ; resolve a device-associated base
...
ff704222 cbz  r4,ff70422e
ff704224 cmp  r4,#1
ff704226 beq  ff704240

; reset type 0: warm
ff70422e ldr  r3,[ff704250]
ff704230 str  r3,[r0,#0x50]
ff704232 movw r3,#0xeca8
ff704236 str.w r3,[r0,#0x1b4]

; reset type 1: cold
ff704240 ldr  r3,[ff704250]
ff704242 str  r3,[r0,#0x50]
ff704244 movw r3,#0xfdb9
ff704248 str.w r3,[r0,#0x1b0]
```

This precisely matches the **operation and reset keys** of Rockchip's U-Boot sysreset driver. The **effective destination** depends on `0xFF706798`'s returned base; the expected RK3288 CRU base is `0xFF760000`, making warm and cold destinations `0xFF7601B4` and `0xFF7601B0` respectively. Recover and verify that base before recording actual effective MMIO addresses as experimentally established.

The full reset request path through `reset_bootloader`, `reset_cpu`, `sysreset_walk_halt`, `sysreset_walk` and driver-model dispatch was already established in §§9–12. The remaining target is the registration/ops-table pointer chain **from sysreset uclass through `0xFF704214`**, not another search for reset magic.

**Why the earlier literal scans missed it:** `MOVW` encodes `0xECA8` and `0xFDB9` in Thumb instructions; neither the complete CRU register addresses nor the warm key need appear as contiguous 32-bit literals. This provides a concrete example that **absence of an exact `BOOT_BROM_DOWNLOAD` word is negative but not exhaustive evidence**.

Reference: https://nest-open-source.googlesource.com/manifest_repos/u-boot/+/06419a4bae2adcdd0ed4e73f93f3f40a82172e45/drivers/sysreset/sysreset_rockchip.c

### 13.3 SPL startup and eMMC U-Boot loading

The Ghidra call graph from the reset vector is:

```text
ff704020 ARM vector -> ff704064 -> ff7040b4 -> ff704068 ARM CPU setup
                                                      |
                                      bl ff7040b8 (cache/MMU handling)
                                      bl ff704118 (thunk toward Thumb)
                                      bl ff705208 (stack/BSS and SPL setup)
                                                       |
                                            indirect branch to ff705350
                                                       |
                                            iterate five boot-device entries
                                                       |
                                            bl ff7054a4 (MMC loader)
```

The `ff705350` decompilation explicitly loops through boot-device candidates, with a direct call to `ff7054a4`. `ff7054a4` initializes MMC, queries boot mode, reads one sector at **LBA `0x100`** using the device's block-read callback, validates an image header and reads additional blocks. This strongly supports **SPL directly loading the p3 U-Boot uImage from eMMC**, rather than always handing off loading to the BootROM. Final read destination, exact header validation, DRAM initialization and branch-to-next-stage remain unproven.

The `ff704120` function should **not** be labelled `save_boot_params` solely because it appears near startup: it references `r9` global state and returns ordinary status codes. Mainline RK3288's SPL-to-ROM continuation capability is optional (`CONFIG_SPL_ROCKCHIP_BACK_TO_BROM`) and has not been demonstrated in the stock InMusic SPL. A generic `BX LR` is not evidence of returning to immutable ROM.

External reference: https://qemu.googlesource.com/u-boot/+/fd6e425be243dce518a02710482514faccf3c211/arch/arm/include/asm/arch-rockchip/bootrom.h

### 13.4 p3 Fastboot has more functionality than the initial reset command

The verified p3 *payload strings* (payload+ offsets) include:

| Feature/clue | Approximate offset(s) | Confidence |
| --- | --- | --- |
| `getvar:` and `download:` prefixes | `0x3F092`, `0x3F09A` | Strings present; dispatcher registration untraced |
| `cb_getvar` | `0x31A5A` | String present; callback link not proven |
| `do_fastboot`, `usb_dnl_fastboot`, `f_fastboot` | `0x31C00`, `0x35166`, `0x37255` | Strings present |
| `fb_mmc_flash_write`, `fb_mmc_erase` | `0x317FD`, `0x31823` | Dangerous MMC handler identifiers; runtime reachability unproven |
| `reboot-bootloader`, `FAILCannot set reboot flag` | `0x32C0D`, `0x32C1F` | String present |
| `inmusic-unlock-magic-...`, `unlock`, `FAILunknown oem command` | `0x32BAB`, `0x32BDA`, `0x32BF5` | Vendor-specific path likely; protection model, arguments and effects unknown. **Do not publish the complete unlock token in logs or this document.** |
| `gpt write mmc %x $partitions` | `0x32B84` | Potential destructive GPT rewrite command, not proven reachable |
| `fastboot_partition_alias_` | `0x39B5D` | Partition alias machinery present |
| `az01_recovercmd`, `az01_bootcmd` | `0x33459`, `0x33491` | Built-in recovery/default boot path |
| `boot_targets=mmc0 mmc1` | `0x3363F` | Built-in target preference |

The existing p3 Ghidra raw import has **mapped memory base `0x00000000`**, but its internal absolute data pointers and uImage load address use **`0x00100000`**. As a result, the Ghidra report displays most Fastboot strings without references. **This is a mapping problem, not evidence those strings are unused.** For proper pointer recognition, create a *separate* raw p3 import at memory base `0x00100000`, auto-analyze it, and compare the results to the original base-0 import. Alternatively, recover raw absolute linked-pointer references with the standalone read-only `mpfree_bootchain_deepmap.py` tool, without changing the Ghidra database.

Safety distinction: a visible `gpt write` or unlock string does **not** authorize issuing USB commands. Especially avoid `oem`, `format`, `flash`, `erase`, `mmc` writes or unlock operations during an initial read-only Fastboot survey.

U-Boot fastboot reference: https://github.com/u-boot/u-boot/blob/main/doc/android/fastboot.rst

### 13.5 Device-tree comparison: ACVA 3.6.0 vs 3.10.0

The offline inventory parsed both:
- 3.6.0 `rk3288-az01-acva.dtb`, under `~/mpfree-work/boot-files/6.6.95-az01-2025-06-27-rt55/`.
- 3.10.0 `rk3288-az01-acva.dtb`, under `~/mpfree-work/boot-files/6.18.44-az01-2026-08-10-rt6/`.

Both define `/power-management@ff730000/reboot-mode` (`syscon-reboot-mode`, offset `0x94`) with:
```text
mode-normal        = 0x5242C300
mode-recovery      = 0x5242C303
mode-bootloader    = 0x5242C309
mode-loader        = 0x5242C301
```
The 3.10.0 ACVA DTB also adds `mode-update=0x5242C301`.

These properties specify **what Linux writes on reboot with the corresponding argument**; they do *not* prove InMusic SPL or silicon ROM interprets all values. In particular, `mode-loader` is **not proof of working Rockusb/MaskROM entry**.

For the ACVA board both DTBs describe `usb@ff580000` as DWC2 peripheral, with `status="disabled"`, and `usb@ff540000` as host. This does not rule out the silicon ROM or U-Boot's own USB stack using the controller; Linux `status` is not an immutable hardware policy. Both DTBs have secure-efuse nodes, but their existence does **not** reveal secure-boot eFuse state.

### 13.6 Version-scoped, non-destructive next pass

1. Run `mpfree_bootchain_deepmap.py` locally on the verified reference `mmcblk0.img`. It reconstructs p3's linked-pointer references to Fastboot strings and contextual command tables without printing unlock secrets or environment values.
2. Run `MPFreeBootChainDecisions.py` in Ghidra, once with p1 and once with p3 open. It prints bounded SPL boot-path, hardware sysreset, and p3 Fastboot pointer-table neighborhoods. It makes **no Ghidra database changes**.
3. Use those reports to map Fastboot callback pointers and exact verification/partition policy, then to trace where SPL's MMC reader loads p3 and jumps into U-Boot.
4. Analyze the embedded **SPL DTB** and **U-Boot DTB** against both ACVA kernel DTBs; confirm sysreset driver registration, boot-source configuration and DWC2 controller bindings.
5. Separately research RK3288 ROM return ABI and USB recovery over public source/TRM. **The BootROM is silicon ROM, not within p1/p3**. Genuine on-device MaskROM behavior requires separately authorized enumeration or hardware evidence.

Scripts are distributed privately, **not committed**. The repository authorization remains restricted to `docs/research/bootrom.md`. No destructive device commands are approved by this section.

### 13.7 Appended device-tree boundary observation

The p3 Ghidra/host reports identify a valid FDT header at raw U-Boot payload offset `0x502F4`, with `totalsize=0x898D`. Their sum is **`0x58C81`**, exactly **363,649 bytes**, the verified U-Boot payload length. Thus the DTB **ends at the exact end of the U-Boot image payload**, a strong structural indication that this build uses U-Boot with an appended DTB (`u-boot-dtb.bin`-style layout). This is an arithmetic observation from verified metadata, not a demonstration of how its internal DTB is selected or relocated at runtime.

The first-stage embedded DTB begins at stage offset `0x627C` and has `totalsize=0xCA7`; it is within the 32-KiB SPL stage and does not extend to its end. Compare the two embedded bootloader DTBs with the 3.6.0 and 3.10.0 ACVA kernel DTBs. The separately distributed offline deepmap tool supports `--extract-embedded-dtbs`, saving private validated FDTs under `~/mpfree-work` without touching the device or Git.

## 14. New owner evidence: actual embedded DTBs and Fastboot callback table (2026-11-10)

**Source reports and private extracted DTBs**:
- `bootchain-deepmap-20261110T141353.txt`
- `bootchain-decisions-p1-20261110T141504.txt`
- `bootchain-decisions-p3-20261110T141612.txt`
- `p1-spl-00-offset-00627c.dtb`, 3,239 bytes
- `p3-uboot-00-offset-0502f4.dtb`, 35,213 bytes

The two FDTs were independently parsed from the owner-supplied binary device trees in this round. They are **private firmware-derived files** and have **not been committed**. The p3 DTB is appended exactly at the end of the p3 uImage payload, as previously established.

### 14.1 SPL vs U-Boot hardware ownership (directly parsed DTBs)

| Property | p1 U-Boot SPL DTB | p3 U-Boot proper DTB |
| --- | --- | --- |
| Total parsed nodes | **14** | **288** |
| `model` | `Akai Professional MPC One` | `Akai Professional MPC One` |
| `compatible` | `inmusic,acva`, `inmusic,az01`, `rockchip,rk3288` | Same |
| `/aliases/mmc0` | `/dwmmc@ff0f0000` | Same |
| `dwmmc@ff0f0000` (eMMC) | `status="okay"`, 8-bit, non-removable | Same |
| `/aliases/mmc1` | Not defined | `/dwmmc@ff0c0000` |
| `dwmmc@ff0c0000` (SDMMC) | Not defined | **`status="disabled"`** |
| `usb@ff580000` (DWC2) | Not defined | **`status="okay"`** |
| `usb@ff540000` | Not defined | `status="disabled"` |
| `/aliases/serial2` | `/serial@ff690000` | Same |
| `/chosen/stdout-path` | `serial2:115200n8` | Same |
| `clock-controller@ff760000` | RK3288 CRU | RK3288 CRU |

**Interpretation:** SPL is purpose-built for the eMMC boot path with only a small pre-relocation device model. Full U-Boot enables DWC2 `usb@ff580000`, explaining the hardware description for U-Boot USB Fastboot even though this board's 3.6/3.10 Linux DTBs disable the same controller. Absence of a USB node in SPL says nothing about **silicon ROM** USB capability, because the ROM does not use SPL's Linux-style FDT.

**Critical installer/recovery qualification:** U-Boot's embedded environment includes `boot_targets=mmc0 mmc1`, but the p3 DTB maps `mmc1` to `/dwmmc@ff0c0000` and marks the device `disabled`. Merely listing `mmc1` **does not prove a working U-Boot SD-card fallback**; U-Boot runtime DT fixups or other path would have to be demonstrated. Neither does this rule out separate SoC BootROM boot-source behavior. Do not use SD fallback as a Tier 3 safety mechanism without direct proof.

Neither embedded p1 nor p3 DTB contains a `syscon-reboot-mode` child node. Kernel DTBs do, with values described in §13.5. The runtime Fastboot writes in p1 and p3 are separate board-specific code; a Linux `mode-loader` property alone does not make a Rockusb/MaskROM entry path in the 2019 U-Boot.

### 14.2 SPL second-stage entry, loader format and unresolved entry policy

The latest p1 report shows the boot loop `0xFF705350` calls the raw MMC loader `0xFF7054A4`, and on a successful load can call `0xFF705310`:

```asm
ff705310 push {r3,lr}
ff705312 ldr  r3,[r0,#0xc]
ff705314 blx  r3
```

The destination is the **entry pointer at image descriptor offset `+0xC`**, not a literal ROM-return call. The image parser `0xFF7052C0` reads and byte-swaps U-Boot legacy uImage `load` (header `+0x10`), `entry` (header `+0x14`) and image-size (header `+0xC`) fields. It *conditionally* fills descriptor `+0xC` either from the uImage entry field or from the load address, depending on descriptor flags at `+0x14`; one branch adjusts load/size to include the 64-byte uImage header. **Therefore the verified p3 header `entry=0` does not mean SPL necessarily jumps to zero**: establish the flag state and the actual populated descriptor before assigning the runtime entry destination.

This path strongly favors SPL-side eMMC loading of p3 in the stock boot sequence. Do not yet assert that all possible BootROM continuation paths are absent.

### 14.3 First Fastboot dispatch-table entries proven from raw linked pointers

The offline deepmap recovered **10,269** aligned words pointing into the p3 load image `0x00100000..`; a contiguous table at raw payload `+0x2F2B0` is consistent with `struct { const char *prefix; handler_fn callback; }` entries of eight bytes each:

| Raw entry offset | Prefix string (linked VA) | Thumb function ptr | Function instruction VA | Confidence |
| --- | --- | --- | --- | --- |
| `+0x2F2B0` | `reboot` (`0x0013F08B`) | `0x00122CF9` | **`0x00122CF8`** | Prefix and linked pointer |
| `+0x2F2B8` | `getvar:` (`0x0013F092`) | `0x00122D39` | **`0x00122D38`** | Prefix and linked pointer |
| `+0x2F2C0` | `download:` (`0x0013F09A`) | `0x00122981` | **`0x00122980`** | Prefix and linked pointer |
| `+0x2F2C8` | Unknown until dereferenced (`0x0013E8AE`) | `0x00122965` | `0x00122964` | Pointer pair, name unresolved |
| `+0x2F2D0` | Unknown until dereferenced (`0x0013F0A4`) | `0x00122949` | `0x00122948` | Pointer pair, name unresolved |
| `+0x2F2D8` | Unknown until dereferenced (`0x0013EF0E`) | `0x00122C5D` | `0x00122C5C` | Pointer pair, name unresolved |

The low bit in each callback pointer encodes **Thumb state**, so the instruction address is pointer & `~1`. A pointer pair strongly suggests command registration, but we still need to trace the dispatcher loop and confirm it invokes the table.

Other important linked references:

- `reboot-bootloader` string is referenced by a literal pointer at payload `+0x22D24`; inspect its enclosing function and state handling.
- OEM/lock-related strings and a vendor-specific marker are referenced around payload `+0x22900..`+0x22944`. The underlying **token value must not be reproduced** in public documentation. Investigate whether this actually gates flash/erase/download/boot and whether any state persists.
- MMC Fastboot write and erase string references occur near payload `+0x1020C` and `+0x10328`; they are leads to code, **not permission for destructive USB commands**.
- U-Boot's `set_reboot_mode` string has a raw pointer around payload `+0x1160`, near `0xFF730094` and `0x5242C300`. Trace this early mode-reader next to determine whether it uses only Fastboot or recognizes other reboot flags.

Relevant upstream Fastboot dispatcher example (functionality differs by version/build):
https://third-party-mirror.googlesource.com/u-boot/+/5e4aeb9b450dbefad6e7b47e39169527d64affa0/drivers/usb/gadget/f_fastboot.c

### 14.4 Correction: last p3 Ghidra "no pointers" report is invalid

The owner ran `MPFreeBootChainDecisions.py` against a Ghidra p3 raw import at block base **zero** and the script reported `Total raw linked-pointer occurrences=0` and many strings as absent. This directly conflicts with the verified offline deepmap that counted **10,269** aligned linked pointers and the earlier literal scanner that found the strings.

**This is a bug in the Ghidra helper's bulk-memory import into a Python bytearray**, not an absence of commands in the binary. The helper used `getBytes(Address, bytearray)` in Jython and then scanned an unpopulated Python buffer. The script must instead pass a proper Java/Jython `jarray.zeros(length, 'b')`, convert each signed Java byte with `& 0xFF`, and validate a known nonzero binary anchor before accepting the result; a per-byte `Memory.getByte` fallback also works.

Separately, mapping p3 at zero hides references because **internal pointers are linked at `0x00100000`**. Recommend importing a *separate* raw p3 program with **memory-block base `0x00100000`**, language `ARM:LE:32:v7` and Auto Analysis. Preserve the previous base-zero program for comparison. Recovered literal/context offsets in the existing reports are **raw payload offsets**; for the new Ghidra import, use `linked VA = 0x00100000 + payload offset`.

### 14.5 Prioritized safe follow-ups

1. Read the six command strings/callbacks starting at `0x0012F2B0` from a correctly mapped p3 program, classify command semantics and note hazards. Recover adjacent callback entries and the dispatch loop.
2. Trace `set_reboot_mode` / early reboot flag consumer near p3 raw `+0x1130`, U-Boot recovery flag setting and reset-mode clearing, and compare with Linux reboot-mode DTB. **This is crucial to test whether `mode-loader` has an actual receiver.**
3. In p1, trace the image descriptor flags in `0xFF7052C0` and indirect `BLX` at `0xFF705314` to determine actual p3 handoff. Confirm read addresses and CRC/image validation.
4. In p1, resolve the sysreset callback registration pointer to the already-confirmed CRU MMIO writer `0xFF704214`, and resolve the base getter `0xFF706798`.
5. No software MaskROM handoff has been proven. Continue reference-source research and board-side USB routing/test-point inventory; **no register writes, firmware modification, Fastboot OEM, flash, erase or hardware reboot is authorized by this document update.**

## 15. Actual eight-entry Fastboot command dispatch resolved (2026-11-10)

The owner ran a **per-byte Ghidra Python Console inspection** of the original 363,649-byte p3 payload, using its verified `0x00100000` linked address convention but without relying on the mis-based Ghidra pointer analysis. It returned **eight consecutive string-pointer/Thumb-callback pairs** at raw p3 offsets `0x2F2B0..0x2F2E8` (eight bytes per pair). This is stronger evidence than strings alone and supersedes the unresolved names in §14.3.

| Command prefix (verified from firmware) | Raw pair offset | Thumb function pointer | Code address (linked VA) |
| --- | --- | --- | --- |
| `reboot` | `+0x2F2B0` | `0x00122CF9` | **`0x00122CF8`** |
| `getvar:` | `+0x2F2B8` | `0x00122D39` | **`0x00122D38`** |
| `download:` | `+0x2F2C0` | `0x00122981` | **`0x00122980`** |
| **`boot`** | `+0x2F2C8` | `0x00122965` | **`0x00122964`** |
| `continue` | `+0x2F2D0` | `0x00122949` | **`0x00122948`** |
| `flash` | `+0x2F2D8` | `0x00122C5D` | **`0x00122C5C`** |
| `erase` | `+0x2F2E0` | `0x00122BF9` | **`0x00122BF8`** |
| **`oem`** | `+0x2F2E8` | `0x00122855` | **`0x00122854`** |

The low bit marks Thumb state. Although the pairs match the expected `struct { char *prefix; void (*cb)(...); }` layout, the dispatcher search loop and its exact matching semantics have **not** been independently decompiled. The table is a concrete code-level registration, but execution reachability, state prerequisites, and permissions still require the function bodies.

### Consequences for a reversible installer

- **Highest-priority target: `boot` at `0x00122964`**, plus `download:` at `0x00122980`. Together they establish that the stock loader has code intended to load an image into memory and boot it. This could allow **RAM-only boot via USB-B Fastboot**, independent of an already-running Linux kernel, while preserving p1/p3/eMMC. It remains unproven whether the firmware accepts our boot-image type, allows it without an unlock, and whether the complete execution path has side effects.
- `continue` at `0x00122948` is conventionally an exit from the Fastboot session back to normal U-Boot boot; it is **not** synonymous with `boot`.
- `flash`, `erase`, and particularly vendor-specific `oem` have potentially persistent/destructive effects. **Do not send these commands for exploratory analysis.** Determine authorization checks and the supported partition aliases from disassembly first. The owner has not approved any such device-side tests.
- `reboot-bootloader` is present as a string/callback neighborhood in p3 but is **not a ninth standalone entry in this eight-row sample**. It may be handled within a prefix callback or separately; determine its exact dispatch path rather than assuming absence or reachability.
- The `oem` subdispatcher warrants a separate *offline* audit of whether there is an InMusic unlock gate, its conditions, and effects, without publishing any embedded unlock marker/token.
- Upstream U-Boot documentation says `boot` normally executes `bootm <FASTBOOT_BUF_ADDR>` unless `fastboot_bootcmd` overrides it. It may require Android boot-image layout and a compatible kernel/DTB/ramdisk. **This is comparison-source behavior**, not yet measured in Akai's 2019 build. https://github.com/ARM-software/u-boot/blob/master/doc/README.android-fastboot

### Next independent analysis

Use the separately distributed `MPFreeFastbootCallbacksAudit.py` inside Ghidra with the raw p3 payload open, ideally from a **second import mapped at `0x00100000` and auto-analyzed** (existing base-zero program remains unchanged). The script validates the table, scans for adjacent entries, records existing callback assembly/call edges/decompilation when available and outputs `~/mpfree-work/fastboot-callback-audit-*.txt`. It is **read-only to Ghidra**, does not contact MPC, and redacts any embedded vendor unlock token. Python syntax validated; not tested in the owner's Ghidra.

Prioritize checking whether `boot` calls `bootm` on the download buffer or invokes vendor logic; whether `continue` resumes Akai's boot sequence; whether `download:` enforces size/bounds; and whether the host USB response-completion callback is needed before the transition. No software MaskROM entry is implied by any of these Fastboot findings.

## 16. Fastboot callback audit identifies deferred USB completion (owner, 2026-11-10)

Private source: fastboot-callback-audit-20261110T144132.txt. All eight previously recorded dispatch pointers still match, and the next table pair at raw +0x2F2F0 begins with a null string pointer. The Ghidra p3 project remained mapped at zero, leaving the callbacks undefined in the Ghidra database; the report's 64-byte machine-code windows were decoded independently as Thumb instructions using LLVM. A lack of Ghidra functions at these locations is not a lack of code.

### Proven immediate boot and continue behavior

The command handlers have matching structures: each loads a global fastboot state pointer at linked address 0x0014D688, then dereferences its field +0x50 (USB request) and writes a Thumb callback pointer to the request at offset +0x10, before tail-branching to the shared response writer at 0x00122734.

| Command | Immediate handler (linked VA) | USB request->complete Thumb pointer | Actual completion instruction VA | Response pointer |
| --- | --- | --- | --- | --- |
| boot | 0x00122964 | 0x001226B5 | **0x001226B4** | 0x00132BA6 |
| continue | 0x00122948 | 0x001226B1 | **0x001226B0** | 0x00132BA6 |

The two immediate handlers **do not themselves perform a kernel jump**. They schedule different post-USB-response callbacks and then invoke a common response writer. The contents of the response string at 0x00132BA6 still require direct reading; the expected protocol response is OKAY, but it is not yet independently verified. What the completion callbacks do, including whether the boot callback invokes bootm or checks the downloaded image, remains unknown until the instructions at 0x001226B0 and 0x001226B4 are examined.

The pattern matches public U-Boot f_fastboot.c, which schedules do_bootm_on_complete for boot and do_exit_on_complete for continue after a successful protocol acknowledgment, but that upstream implementation does not prove the actual InMusic callback bodies: https://qemu.googlesource.com/u-boot/+/b6c50e5831f6ce3800d4b3cf3c7aa35dde8c48d9/drivers/usb/gadget/f_fastboot.c

### OEM and storage behavior

The OEM handler at linked VA 0x00122854 begins by reading a command buffer pointer, advancing four bytes past the OEM prefix, and preparing a six-byte comparison with a string pointed to by a literal. The compared string has **not been verified**; do not infer that it is format, unlock, or any other subcommand. The rest of the branch tree is absent from the submitted 64-byte window. Flash, erase, and download have longer implementations; their authorization, bounds, and persistent effects remain unproven. Do not send state-changing Fastboot commands for exploratory analysis.

### Next exact targets

1. Import a **separate** raw p3 payload into Ghidra at initialized block base 0x00100000 with ARM little-endian v7 and run Auto Analysis. The old base-zero import was never fixed in the last round.
2. Decode 0x001226B0, 0x001226B4 and 0x00122734 and follow immediate callees. Confirm the post-ack boot and continue effects, response contents and any validation or persistent-state changes.
3. Trace 0x00122980 download buffer location, maximum transfer size and image format. Only after that should a RAM-only boot proposal be treated as a testable safe installer path.
4. Keep investigating the OEM and storage branches **offline**, and do not publish vendor token bytes.

No changes to firmware/device or other Git files are authorized by this section.

### 16.1 Shared Fastboot precondition helper at 0x00122834

Further decoding of the same 64-byte callback windows reveals that **download, flash, erase, and one conditional branch of OEM** each call the same Thumb helper at linked VA **0x00122834**. The direct BL targets were computed from the instruction offsets in the owner report: download +0x2298C, flash +0x22C64, erase +0x22C00, OEM +0x2286A. The caller branches on its result. This is a high-value new candidate for a shared permission or state check, but its actual semantics are **unknown** until its function body is decoded; do not label it an authorization gate yet. Analyze this helper alongside the deferred boot and continue USB completion callbacks.

## 17. Offline Thumb decoding: Fastboot RAM boot and volatile operation gate (owner, 2026-11-10)

The owner ran the read-only, SHA256-verified p3 code extractor and supplied private `fastboot-completions-hex.txt`. The payload hash matched `b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a`. Relevant code windows were decoded independently using the local LLVM ARMv7 Thumb disassembler with linked addresses; addresses and call edges below are measured from **the 2019 InMusic binary**, while symbolic names are comparison-source inferences.

### 17.1 USB completion stage: actual bootm-shaped invocation

`continue`'s post-response completion handler is at linked `0x001226B0` and contains a **direct unconditional Thumb branch to `0x00121DD0`**. The destination is not yet examined; it is consistent with a USB detach / resume path but must not be assumed.

The `boot` post-response completion handler at **`0x001226B4`** is longer. It prepares a three-element array on the stack, with the last element NULL, and then:

```asm
001226B4 ldr  r3,[pc,#0x3c]      ; literal 0x0013B8B4, candidate "bootm" string
001226BA ldr  r0,[pc,#0x3c]      ; literal 0x0013467B, string not decoded
001226BE str  r3,[sp,#0x0c]      ; argv[0]
001226C0 str.w sp,[sp,#0x10]    ; argv[1] points to formatted stack buffer
001226C4 str  r4,[sp,#0x14]      ; argv[2] = NULL (r4 = 0)
001226C6 bl   0x0010D14C
001226CA ldr  r3,[pc,#0x30]      ; literal 0x00143E34, global value address
001226CC mov  r0,sp
001226CE ldr  r1,[pc,#0x30]      ; literal 0x0013468D, format string not decoded
001226D0 ldr  r2,[r3]           ; value of global, likely fastboot_buf_addr
001226D2 bl   0x0012E950      ; likely print/format the address
001226D6 add  r3,sp,#0x0c     ; argv for boot
001226D8 mov  r1,#0
001226DA mov  r2,#2
001226DC mov  r0,#0
001226DE bl   0x00101744      ; extremely strong do_bootm-like call signature
001226E2 mov  r3,#0
001226E4 mov  r2,#0
001226E6 mov  r1,#0
001226E8 mov  r0,#0
001226EA bl   0x0011622A      ; already measured reset_cpu-like wrapper
```

Conclusions:
- Stock Fastboot **intends to boot the downloaded image from memory using a bootm-style command invocation** after sending `OKAY`. The direct callback invokes `0x00101744` with `(cmdtp=NULL, flags=0, argc=2, argv)`, very strong structural evidence for `do_bootm()`. It then calls the known reset wrapper if execution returns.
- The global at **`0x00143E34`** supplies a value used in the boot command string and is a candidate for `fastboot_buf_addr`; confirm its initializer and whether the download callback writes to exactly that address. The strings at `0x0013B8B4`, `0x0013467B`, and `0x0013468D` need direct reading to finalize argument semantics.
- The shared Fastboot response literal `0x00132BA6` is **verified to be "OKAY"**, not merely inferred.
- The current data do not prove whether the image must be Android-style, legacy uImage or FIT, whether memory regions overlap a running loader, whether a `fastboot_bootcmd` override is present, or whether boot succeeds on actual hardware. Do **not** send any device-side boot/download commands yet.
- The assembly matches an upstream U-Boot implementation where Fastboot boot constructs argv for `do_bootm` and calls reset on failure. Treat reference source as guidance, not exact provenance: https://nest-open-source.googlesource.com/manifest_repos/u-boot/+/06419a4bae2adcdd0ed4e73f93f3f40a82172e45/drivers/fastboot/fb_common.c

### 17.2 Actual shared gate and OEM state setter

The `download`, `flash`, `erase`, and a branch of `oem` command handlers call `0x00122834`. This helper has now been decoded:

```asm
00122834 ldr  r3,[pc,#0x14]  ; [0x0012284C] = 0x0014D690
00122838 ldr  r4,[r3]        ; test global operation flag
0012283A cbnz r4,0x00122846
0012283C ldr  r0,[pc,#0x10]  ; [0x00122850] = 0x00138FE8, not yet decoded
0012283E bl   0x00122734     ; transmit a Fastboot response if flag is zero
00122842 mov  r0,r4          ; return 0
00122846 movs r0,#1          ; return 1 if flag nonzero
```

Thus **`0x0014D690` is a global permission/state flag**, although its startup value and other writers have not yet been fully traced. The OEM callback at `0x00122854` reveals that:
1. It recognizes `"format"` by comparing six bytes with the received subcommand (private report explicitly verified that string).
2. On that branch, it **calls `0x00122834`**, and when the flag permits it, constructs a command using the embedded GPT-writer format string and calls a U-Boot command-execution-shaped function `0x00110484`. This is a **potentially destructive partition-table rewrite path** and must never be used as a discovery command.
3. It compares another subcommand against a **37-byte vendor-specific string** (the value is deliberately omitted), and on a match executes `str 1, [0x0014D690]` before emitting an apparent success response. This firmly links the OEM marker to the permission/state flag. **Do not publish the marker itself.**
4. Other nearby OEM comparisons and effects, including a possible `resetenv` branch, need separate verification. No on-device requests have been made.

The fact that a recognized OEM string writes the flag does **not** demonstrate the flag's persistence: the visible instruction is a RAM write, not an eMMC write. Nor is this by itself evidence of a secure hardware root of trust. The exact `FAIL...` response when the gate is zero remains to be read at `0x00138FE8`.

### 17.3 Priority next analysis

- Resolve the **`0x00121DD0`** continuation path, classify whether it detaches USB and returns to the built-in `az01_bootcmd`.
- Resolve the **`0x00143E34`** download-buffer pointer, length limit and overlap with U-Boot/DTB; verify strings in the deferred boot callback.
- Locate the origin/initialization of **`0x0014D690`** and classify any other readers/writers; evaluate whether all affected commands are reliably blocked until the OEM state changes.
- Trace `0x00101744` and `0x00110484` far enough to confirm `do_bootm` and U-Boot `run_command`, respectively.
- Separately continue tracing p3 reboot-mode reader and p1's ROM-return paths. Nothing in these Fastboot findings proves any MaskROM software entry.

Scripts used so far have only read the private local backup; **no changes to the MPC, emmc, firmware or other repository files**.

## 18. Verified Fastboot bootm, download window and continue flag (2026-11-10)

Owner-supplied `fastboot-next-trace.txt` was produced by reading only the known p3 uImage from the previously verified offline eMMC backup. The payload's SHA-256 was checked against `b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a`. The selected Thumb instruction windows were decoded independently for this report. **These are static findings; no USB boot test has been performed.**

### 18.1 Definitive boot-related strings and candidate buffer address

The report directly reads these exact strings from the p3 binary:
- `0x0013B8B4` = `bootm`.
- `0x0013467B` = `Booting kernel..\n`.
- `0x0013468D` = `0x%lx` (formats the RAM address).
- `0x00138FE8` = `FAILpermission denied`.
- `0x00132BA6` = `OKAY`.

The **file bytes** located at linked VA `0x00143E34` decode to `0x00800800` (an 8-MiB-plus-2-KiB RAM address). The deferred `boot` USB completion function `0x001226B4` has already been shown to assemble an argument vector and call `0x00101744` with the U-Boot command signature `(cmdtp, flags, argc=2, argv)`, then call the CPU reset wrapper if the boot operation returns. With the exact `bootm` and address-format strings now verified, **this is compelling binary-level evidence for Fastboot RAM-boot implementation**, not proof that a given downloaded MPFree image will boot.

### 18.2 Download command and USB receive callback

The actual `download:` callback at **`0x00122980`**:

1. Calls shared gate `0x00122834` and exits on a zero return; the gate's failure response is confirmed to be **`FAILpermission denied`**.
2. Parses the request size using a conversion helper called with **base 16**, consistent with Fastboot's hexadecimal download length.
3. Writes zero to the receive-count global at **`0x0014D680`** and writes requested size to **`0x0014D684`**.
4. Checks `size <= 0x70000000`, i.e. at most **1.75 GiB**; this is a protocol acceptance limit, **not a memory safety guarantee**. A zero-size or oversized request follows an error/response path. The exact further error text remains unclassified.
5. Installs Thumb completion callback **`0x00122A11`** (instruction address `0x00122A10`) into the USB request at field offset `+0x10` and prepares a Fastboot `DATA...` reply.

The USB receive callback at **`0x00122A10`** (partially available in report) reads USB request status, `actual` length, the cumulative byte count and expected length. It limits each memcpy-shaped transfer to remaining requested bytes, and constructs its destination from:

```text
destination = 0x00800000 + 0x00000800 + bytes_received
            = 0x00800800 + bytes_received
```

The code calls a memcpy-shaped routine at `0x0012DDA2` with source `request->buf`, destination as above and the bounded length. It increments the byte count at `0x0014D680` and later checks whether all requested bytes have arrived. The tail of the callback (`0x00122A80` onward) was **not** included; final USB reply, requeue logic, zero-length handling and failure behavior still need to be traced.

**Safety:** the 1.75-GiB request bound is far too broad to treat as a safe test-image limit without a complete RAM map and verified reserved regions. Physical memory capacity alone does not establish free RAM, the range must avoid U-Boot code/data/stack, DTB, secure carveouts and other DMA buffers. No on-device download or boot experiment should be attempted from this evidence alone.

### 18.3 Continue callback actually sets the gadget exit flag

The previously identified `continue` post-response callback is **`0x001226B0`**, an unconditional Thumb branch to **`0x00121DD0`**. The latter function is now decoded:

```asm
00121DD0 ldr  r3,[pc,#4]    ; literal [0x00121DD8] = 0x0014D658
00121DD2 movs r2,#1
00121DD4 strb r2,[r3]
00121DD6 bx   lr
```

An adjacent sibling at `0x00121DDC` writes zero to the **same byte**. The interpretation that `continue` requests exiting the Fastboot gadget session is strongly supported, but the actual USB polling loop's reader of `0x0014D658` is still to be traced. The callback itself does **not** reset the CPU or enter MaskROM.

### 18.4 Important correction about the permission flag's “initial value”

Reading the **on-disk payload bytes** at linked VA `0x0014D690` yields `0x00000017`, but **that does not establish that the running U-Boot gate starts enabled, locked, or set to 23**. Relocation, BSS/data initialization, code/data mapping and other startup writes have not been reconstructed. The *runtime* shared check at `0x00122834` compares the actual value stored at that address to zero, and the vendor OEM branch writes `1` to the same global. Until the runtime initialization and all writers are traced, make **no assertion about default lock/unlock state**. The apparently unusual raw value 23 is precisely why the distinction matters.

### 18.5 Next proof targets

- Trace `0x00122A80` onward to recover final transfer success/failure, actual maximum safe ranges and USB requeue behavior.
- Resolve xrefs and runtime initialization for `0x0014D680`, `0x0014D684`, `0x0014D690`, `0x00143E34`, and `0x0014D658`.
- Confirm image types supported by `0x00101744` (bootm-like), the validated `bootm` arguments, any boot-signature checks, and device-tree/ramdisk handoff.
- Preserve distinction between U-Boot Fastboot USB RAM boot and genuine RK3288 silicon MaskROM recovery. The former is now strongly supported by the binary; the latter remains unproven.
- No live Fastboot `download`, `boot`, `flash`, `erase`, `oem` or ROM register experiments have been performed.

## 19. Offline Fastboot transfer completion and bootm image-format trace (owner, 2026-11-10)

New owner report: private `fastboot-boundaries-next.txt`, extracted after SHA256 verification of the same 2019 p3 U-Boot raw payload (`b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a`). The selected ARMv7 Thumb byte windows were decoded offline with LLVM. This section is *measured from the provided binary* unless a claim is labelled inference. No live USB or flash operation occurred.

### 19.1 Fastboot request and response text

Explicit firmware strings: `Starting download of %d bytes`, `FAILdata invalid size`, `FAILdata too large`, and `DATA%08x`. These confirm the expected Fastboot size-validation and positive DATA handshake strings. The separate gate failure is `FAILpermission denied` (§18). Their presence alone does not establish which branch generates each error, but the earlier download handler disassembly showed a base-16 size parser and size bound `0x70000000` (1.75 GiB). The latter is **a protocol limit, not a safe memory map**, and the board's actual usable RAM carveouts still need separate verification.

### 19.2 Receive callback termination branch

The existing USB receive callback (entry `0x00122A10`) includes the following measured instructions in its completion path:

```asm
00122A74 ldr   r2,[r6]        ; byte count state
00122A76 ldr.w r3,[r8]        ; requested-size state
00122A7A cmp   r2,r3
00122A7C blo   0x00122AB4   ; not yet complete
00122A7E movs  r3,#0
00122A80 str.w r3,[r8]       ; clear pending-size state
00122A84 ldr   r3,[pc,#0x40] ; pointer literal 0x00122781 (Thumb)
00122A8A str   r3,[r5,#0x10] ; USB request complete callback
00122A8C mov.w r3,#0x1000
00122A90 str   r3,[r5,#0x4]  ; USB request length 4096
00122A92 bl    0x0012DBF4   ; formats a reply; argument string needs confirmation
00122A98 bl    0x00122734   ; shared Fastboot response writer
00122AAE ldr   r3,[r3,#0x10]
00122AB0 blx   r3            ; invoke USB request operation via driver pointer
```

At `0x00122AB4` the incomplete-transfer branch calls `0x0012247C`, stores its result as the next request length, and rejoins the USB request submission path. The successful-transfer branch acknowledges the completed download and resets transfer state; the exact response string can be established from the literal pool by a focused next pass. This is an executed-code-path finding, not merely a recognized `download:` string.

### 19.3 Legacy uImage handling in the built-in bootm command

The `do_bootm`-shaped entry at `0x00101744` dispatches a boot operation using parsed address/arguments and delegates work to `0x0010A6A4`. A nearby image-format handler at `0x001017FC` calls `0x0010E390` and branches based on format codes 1 and 2. On the code-1 path, it compares the image's first word against the little-endian representation `0x56190527` of big-endian legacy uImage magic **`0x27051956`** (visible as literal bytes `27 05 19 56`), then invokes additional validation/boot helpers (`0x0010E000`, `0x0010E1CC`, `0x0010E02A`, etc.). The code-2 path invokes helpers `0x0010F49C`, `0x0010F728`, `0x0010F388` and may represent FIT/FDT handling, but its exact constraints are unproven.

**Interpretation:** Legacy U-Boot image recognition is strongly supported by the machine code. Do **not** yet claim the Fastboot path boots any arbitrary legacy uImage, that a FIT image is supported, that all CRC checks were identified, or that the MPFree kernel/ramdisk/DTB handoff will succeed. Boot-image load address, entry point, architecture/type, decompression, RAM placement and DTB compatibility still require scrutiny before live testing.

### 19.4 Verified Fastboot exit state reader

`0x00121DC4` reads a byte from the global at `0x0014D658` and returns it. `0x00121DD0` sets that byte to 1, while `0x00121DDC` sets it to 0. The previously measured Fastboot `continue` USB completion callback branches to the setter. This is a coherent exit-request/read/clear design, although the main event loop caller that polls this helper remains to be traced.

### 19.5 Runtime state is not derived from raw file words

The offline scan found the on-disk words at `0x0014D658` and `0x0014D690` equal `0x00000017`, and the on-disk word at `0x0014D684` equals `0x001304A4`. These **are not trustworthy runtime initial values** of the exit flag, permission flag, or download size: the image may have startup initialization, BSS writes, relocation, and nontrivial memory layout. The gate at `0x00122834` *at runtime* checks the current word at `0x0014D690`, and an OEM branch sets it to 1. Its actual power-on state is still unknown. Two observed literal pointers refer to that permission-global address, at raw code+`0x2284C` and code+`0x22928`; the absence of other full-address literals is not proof there are no computed references.

### 19.6 Next research before any live test

- Trace the USB response sequence and `0x00122780` completion callback; verify the positive download response and the final copy/sync behavior.
- Reconstruct U-Boot's actual RAM bank/reserved region/stack/relocation limits and memory-buffer size, not merely the `0x70000000` protocol bound.
- Identify `0x0010E390` image classification and the two image-format branches, including the legacy uImage CRC, load-address, entry-point and DTB requirements.
- Recover runtime initialization and all callers of the permission-gate check and exit-flag reader. Do not assume USB downloads are accessible in the default state.
- Continue separate Rockchip silicon BootROM/MaskROM study; nothing here proves an SPL-to-MaskROM handoff.

No MPC reboot, USB Fastboot traffic, eMMC writes or permission-changing commands were performed in this phase.

### 19.7 Independently parsed U-Boot DTB confirms 2 GiB physical memory (private p3 FDT)

The owner supplied private `p3-uboot-00-offset-0502f4.dtb` (35,213 bytes). It was independently parsed here from its FDT structure and string blocks; this is not inferred from a board datasheet. Its `/memory@0` node has `device_type="memory"` and `reg=<0x00000000 0x80000000>`, with 1 address cell and 1 size cell at the root. Thus the *declared physical DRAM bank* spans `[0x00000000, 0x80000000)` = **2 GiB**. The FDT's memory reservation map has no entries, but this does **not** establish that U-Boot has no runtime reserved allocations or relocation/stack regions.

The Fastboot download buffer base from p3 code is `0x00800800`; the source-level size-bound from the download callback is `0x70000000`, placing the theoretical end at **`0x70800800`**, still **`0x0F7FF800` (~248 MiB)** below the declared physical RAM limit. This arithmetic is consistent with an intentional high-memory margin, but the *actual safe writable area* is unproven. The maximum must **not** be used to size an experimental MPFree image. The proper next gate is a runtime U-Boot memory map/relocation/stack/heap/USB DMA allocation review plus an independently verified small image.

## 20. Fastboot command dispatch proven; legacy image CRC and FIT identification (2026-11-10)

New owner report `fastboot-image-validation-next.txt` contains SHA256-verified p3 U-Boot Thumb machine-code windows at linked addresses `0x00122734`–`0x00122820`, `0x0010E350`–`0x0010E3D0`, `0x0010DFD0`–`0x0010E080`, `0x0010E1A0`–`0x0010E240`, and `0x00121C30`–`0x00121DD0`. The machine bytes were decoded using LLVM as **Thumb**. The ASCII strings independently verified in the report are `OKAY`, `downloading of %d bytes finished`, `bootm`, and `## Checking Image at %08lx ...`. These observations refer to stock InMusic U-Boot 2019, not newly compiled MPFree.

### 20.1 The dispatcher actually walks the eight-element Fastboot callback table

The function at **`0x00122780`** reads the incoming command buffer and searches the table whose linked address is **`0x0012F2B0`** (literal at `0x00122818`). It performs a prefix-comparison call at **`0x001227A4 -> 0x00122710`**; on a miss it advances by one entry and tests **`index != 8`**. On a match it loads the callback from entry offset `+4` and invokes **`BLX r3` at `0x001227E6`**. Its caller/request setup includes buffer-length checks and requeues a USB request afterwards. This **confirms control-flow connectivity to the eight-entry table**, not just data registration. The actual incoming-command prefix match helper can be inspected separately, but the loop's bounded table scan and indirect dispatch are unambiguous.

The eight registered commands from §15 remain: `reboot`, `getvar:`, `download:`, `boot`, `continue`, `flash`, `erase`, `oem`. No additional registered table row was found after the null-terminated eighth entry in prior reports. OEM has additional internal subcommands.

### 20.2 Response submission to USB endpoint

`0x00122734` calls a length helper at `0x0012DCB2` on the supplied Fastboot response, copies that many bytes into a USB request buffer through `0x0012DDA2`, records the request length in `[request+4]`, and calls USB endpoint operations indirectly (`BLX r3` at `0x0012275A` and `0x00122768`). It returns zero after queueing, with an error diagnostic branch. This establishes a concrete connection between the Fastboot textual ACK/FAIL responses, their packet buffers and endpoint activity; the specific method names behind endpoint operation slots require separate verification.

### 20.3 Format classifier at 0x0010E390

The binary performs `ldr r2,[r0]`, compares against an inline literal **`0x56190527`**, then returns **1** for a legacy uImage. If the magic does not match, it invokes **`0x00127A1C`** on the candidate image, returning **2 when that function returns zero**, otherwise **0**. Structurally it matches upstream U-Boot `genimg_get_format()` using `image_check_magic()` and `fdt_check_header()`. Thus **format 2 is a likely FIT/FDT-based format**, but the binary call at `0x00127A1C` still needs disassembly before claiming an exact `fdt_check_header()` implementation or acceptance of a FIT payload by `bootm`.

Upstream comparison source (not proof of its implementation being identical): https://android.googlesource.com/platform/external/u-boot/+/f7aa2094d4b1f8121384a030e95177327d873339/common/image.c

### 20.4 Image-check functions confirmed by byte-level control flow

**`0x0010E000`: legacy uImage header CRC.** Copies a **64-byte header** into stack storage (`BL 0x0012DE1C`), writes zero at copied header offset `+4` (CRC field), calls a CRC-like routine `0x0012DA74` with seed zero, buffer pointer and length `0x40`, then compares the return value with the original header's big-endian stored CRC (`REV` before compare). This precisely matches upstream `image_check_hcrc()`.

**`0x0010E02A`: legacy uImage data CRC.** Reads the big-endian image data size at header offset `+0x0C` (byte-swaps it), takes the data pointer as image header `+0x40`, calls CRC-like routine `0x0012DA80` with seed 0, buffer, length and chunk-size parameter `0x10000`, then compares with byte-swapped original header offset `+0x18` (data CRC). This precisely matches upstream `image_check_dcrc()` with watchdog chunks. The CRC algorithm itself is structurally inferable as CRC32 but the callees have not been separately disassembled in this report.

**Execution qualification:** these functions exist and implement the expected checks, but the currently supplied windows **do not prove that every Fastboot `boot` invocation necessarily runs both CRC functions** or every other image-policy test. Callers must still be traced within the bootm image-loading chain.

**Correction:** `0x0010E1CC`, previously labelled a generic 'additional image validation' target, is **not shown to validate the image**. Its Thumb code reads the uImage name string from header offset `+0x20` and converts header bytes `+0x1C..+0x1F` (OS, architecture, type, compression) into printable names. It prints size, load and entry-address fields from offsets `+0x0C`, `+0x10`, and `+0x14`. This is strongly consistent with **`image_print_contents()`**, an informational printer, not a trust/CRC gate. Do not conflate it with validation routines.

Upstream reference for `image_check_hcrc`, `image_check_dcrc`, and `image_print_contents`: https://github.com/ARM-software/u-boot/blob/master/common/image.c

### 20.5 USB event-loop boundary

The uploaded `0x00121C30..0x00121DD0` window contains USB controller/request setup, endpoint registration and transfer operations. The previously proven flag getter `0x00121DC4` loads a byte from `0x0014D658`, `0x00121DD0` sets it, and `0x00121DDC` clears it; the `continue` post-response callback branches to the setter. This window still **does not identify the upper-level Fastboot USB event-loop caller** that polls the flag and resumes the Akai boot sequence. Do not assert a specific return behavior without the caller.

### 20.6 Revised research priorities

1. Disassemble **`0x00127A1C`** to establish whether it is `fdt_check_header`, and trace the consumers of format values 1/2/0 within bootm.
2. Trace **callers of `0x0010E000` and `0x0010E02A`**, inspect bootm orchestration near `0x0010A6A4`, and enumerate architecture/OS/image-type/CRC/security checks actually applied to a Fastboot-downloaded kernel.
3. Reconstruct live RAM placement/relocation, USB DMA and stack so the protocol maximum `0x70000000` is not confused with safe download capacity.
4. Determine runtime initialization of permission gate `0x0014D690` and all writes; static file word values are not live state.
5. Track the upper-level USB polling loop reading the exit flag at `0x0014D658`, and keep ROM/MaskROM investigations independent. No live USB requests, eMMC writes, reboots, or OEM commands were performed.

## 21. Two concurrent tracks: p3 bootm call graph and silicon RK3288 MaskROM recovery (2026-11-10)

**Owner input:** `bootm-validation-calls-next.txt` from the verified p3 payload (`b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a`). This round also cross-checked upstream Rockchip/U-Boot code, the 2015 RK3288 TRM, RK3288 board reference manuals, and earlier verified p1 Ghidra reports. Sources are separated below. No device commands were issued.

### 21.1 Track A — the actual bootm orchestration invokes uImage CRC checks

Raw Thumb `BL` target scanning identifies code call sites within U-Boot's `bootm` image orchestration routine near `0x0010A6A4`: `0x0010A758 -> 0x0010E390` (format classifier); on the legacy-uImage branch `0x0010A7A6 -> 0x0010E000` (header CRC), `0x0010A7C2 -> 0x0010E1CC` (metadata printout), and `0x0010A7D0 -> 0x0010E02A` (data CRC). These addresses are consistent with the adjacent ARM Thumb machine-code windows supplied in the report. The caller checks the CRC helpers' Boolean return values and takes error branches on failure.

The previously unidentified second image format is now highly confidently **FDT-header based**: the callee at `0x00127A1C` reads a candidate image word, byte-swaps it with `REV` and compares it to the literal `0xD00DFEED` stored around `0x00127A60`, then checks other big-endian FDT header fields and returns specific errors. This matches `fdt_check_header` style behavior. A valid FDT *header* supports FIT image identification, but does not prove a particular FIT configuration can boot or that every FIT signature policy is enforced.

These are static code-flow findings: the firmware contains a viable code path **Fastboot completion -> bootm -> image classifier -> legacy image header/data CRC checks**. Remaining: image OS/architecture/type/compression policy, signature/security rules, load/entry address safety, Linux DTB/ramdisk handoff, runtime Fastboot permission initialization and safe USB download-buffer size.

### 21.2 Track B — Rockchip has a genuine immutable BootROM USB mode

The RK3288 *silicon* BootROM is **20 KiB** according to the RK3288 TRM (2015, chs. 1–2), and the initial ROM mapping uses **`0xFFFF0000`**. The SoC supports ROM remapping controlled by **`SGRF_SOC_CON0[11]`**; therefore the startup mapping is **not a promise that software can read the same physical location later**. The ROM is not contained in any p1/p2/p3 eMMC backup or Akai rootfs, and no silicon ROM dump has been acquired.

The RK3288 TRM System Boot flow checks NAND, eMMC, SPI and SD boot media for a valid ID block before falling back to USB OTG. In true ROM USB mode it downloads DDR initialization code into SRAM, runs it, then downloads a next-stage loader to initialized RAM. A separate phyCORE-RK3288 manual confirms boot-order-based USB fallback and describes board-specific hardware boot-device disable signals. **Those signals belong to PHYTEC's board**, and MUST NOT be assumed to exist at the same positions on MPC One.

**Critical control-flow distinction:** upstream Rockchip U-Boot's `bootrom.c` explicitly says that on its supported return-to-ROM flow, the silicon BootROM does **not itself inspect** `BOOT_BROM_DOWNLOAD=0xEF08A53C` in the reboot-mode register. **Early TPL/SPL** checks and clears that value, then returns via the saved ROM context with a nonzero result to request USB download mode. Writing only the magic and resetting without this early-stage handler can simply proceed through the normal boot path. See upstream source below. This explains why the modern Linux reboot-mode flag is not sufficient evidence that stock Akai firmware supports software entry.

The stock InMusic p1 image has **no exact 32-bit literal** `0xEF08A53C`, and its startup sequence `0xFF70409C -> 0xFF704118 -> 0xFF7091F8` is resolved by earlier Ghidra analysis to a short thunk ending at `0xFF7041E4: BX LR`. This appears to be a **no-op early hook**, not the `setjmp/longjmp` ROM-context preservation expected by upstream `CONFIG_SPL_ROCKCHIP_BACK_TO_BROM`. It is **strong negative evidence** for the stock SPL's software-return mechanism, but not a mathematical proof of absence of every possible computed/alternate entry. The normal p1 SPL also loads p3 directly from MMC and enters it via the image descriptor rather than invariably returning to silicon ROM.

Rockchip's known BootROM fallback when no valid external ID block is found is independent of p1/p3, but it still requires: (a) the MPC's USB OTG controller to be electrically connected to a usable external device-mode connector, (b) an authorized/non-destructive way to make the ROM take the USB path, (c) a compatible DDR loader and USB-stage code, and (d) success under any fuse/security policy. The p3 embedded U-Boot DTB has DWC2 `usb@ff580000` enabled, and public RK3288 DTS identifies this as the SoC's USB OTG controller. **This is a strong routing clue, not electrical proof that the MPC USB-B connector exposes ROM mode.**

Rockchip boot-source IDs (from upstream bootrom.h) include eMMC=2, SD=5 and USB=10; these are ROM boot-source identifiers, **not** `mode-loader=0x5242C301`, Fastboot or USB VID/PID numbers.

### 21.3 Recovery strategy and safety tiers

1. **Proven development path:** original Linux -> kexec 3.10.0 / MPFree testing, without modifying p1/p3. Stock U-Boot USB Fastboot RAM boot is a **strongly evidenced** additional path, pending permission-state, image-policy and memory-overlap checks.
2. **Host-observation-only mode survey:** with a USB cable already connected and without rebooting, inspect Linux host `lsusb`, `lsusb -d 2207:` and, if installed, `rkdeveloptool ld` (list devices only). Rockchip USB vendor ID is `0x2207`; enumeration alone is not proof of MaskROM versus a U-Boot Loader/Fastboot variant. No device present in normal boot proves nothing about ROM mode.
3. **Board investigation:** identify MPC's RK3288 USB OTG data path to the USB-B connector and inspect for board-specific boot-recovery pads/buttons/straps from photos, schematics or trusted measurement. **Do not** recommend grounding a random eMMC pin, erasing an ID block, or resetting into untested ROM mode.
4. **Independent recoverability threshold:** controlled ROM enumeration, verified loader compatible with the particular MPC board's DRAM setup, read-back verification against the known eMMC backup, and a separately approved restoration plan. Only once this path is demonstrated can installer design safely consider writing boot-critical partitions.
5. **ROM code research:** consult the RK3288 TRM (boot addresses/remap) and public coreboot/RK3288 implementations. A silicon-ROM dump would require a specifically validated read-only ROM-access path or authorized hardware debug; no generic `/dev/mem`, debug-port or register-writing commands are approved. Public `rkdeveloptool` source and the RockUSB protocol are independent references; the tool's `db`, `wl`, `ef`, `rd`, `ul` operations are **not** read-only investigations.

### 21.4 External sources consulted

- RK3288 technical reference manual, part 1, chapter 2, boot sequence and remap: https://www.rockchip.fr/Rockchip%20RK3288%20TRM%20V1.0%20Part%201-System%20and%20System%20Control.pdf
- U-Boot RK3288 return-to-ROM description: https://android.googlesource.com/platform/external/u-boot/+/218bb2ca2b66e646267a56af52acb8e4cc5b0bc9/doc/README.rockchip
- Rockchip `bootrom.c` with early SPL flag check and explicit note ROM does not check flag: https://coral.googlesource.com/u-boot-mtk/+/refs/heads/master/arch/arm/mach-rockchip/bootrom.c
- Rockchip bootrom command enums and boot-source IDs: https://qemu.googlesource.com/u-boot/+/fd6e425be243dce518a02710482514faccf3c211/arch/arm/include/asm/arch-rockchip/bootrom.h
- RK3288 reference board manual (boot order and board-specific boot-disable wiring): https://www.phytec.eu/fileadmin/legacy/downloads/Manuals/L-826e_1.pdf
- USB MaskROM RAM-loader stages, upstream U-Boot discussion (examples for other Rockchip SoCs, not proof of MPC compatibility): https://lists.u-boot-project.org/pipermail/u-boot/2025-May/589194.html
- Rockchip rkdeveloptool: https://github.com/rockchip-linux/rkdeveloptool

**Status:** Fastboot `bootm` and legacy CRC reachability are now evidenced; real, on-chip RK3288 MaskROM USB recovery exists by design but has **not been shown accessible/recoverable on the MPC One board**. The two tracks will remain separate until board-specific evidence connects them.

## 22. USB host inventory: no device enumerated in normal Linux (2026-11-10)

**Owner-observed workstation output:** the Linux host ran `lsusb`, `lsusb -d 2207:`, and checked for `rkdeveloptool`. Host USB inventory included Logitech peripherals, a VIA hub, an Anker camera, Intel Bluetooth and AMI virtual devices; **there was no detectable Rockchip VID `0x2207` and no recognizable MPC One USB device** in that host snapshot. `rkdeveloptool` was **not installed**, so `rkdeveloptool ld` did not run. The owner did **not** explicitly confirm a direct data-capable USB cable from MPC USB-B to the workstation at the time; no claims about MaskROM or board-level USB routing can be concluded from the snapshot.

**Interpretation by mode:** an MPC running stock Akai Linux need not enumerate as a USB peripheral unless Linux/Controller Mode has configured the gadget. Absence of USB device in this state is **not** evidence that U-Boot Fastboot or true silicon MaskROM is unavailable. Installing `rkdeveloptool` alone cannot create a missing USB enumeration event. Even a matching Rockchip VID could represent Loader rather than silicon MaskROM and must be classified separately.

**Verified external sources:**
- Akai explicitly documents the MPC One's **USB Type-B port** as its PC-facing connection, and says the shipped cable is USB 2.0; USB-A is for host peripherals: https://support.akaipro.com/en/support/solutions/articles/69000816149-akai-pro-mpc-one-frequently-asked-questions
- Akai says the MPC can expose computer-controller mode over USB-B, when configured appropriately on the MPC: https://support.akaipro.com/en/support/solutions/articles/69000824436-akai-pro-mpc-series-transferring-content-to-and-from-the-mpc-via-a-computer
- Rockchip project's udev rules list **RK3288 VID:PID `2207:320a`**: https://github.com/rockchip-linux/rkdeveloptool/blob/master/99-rk-rockusb.rules
- Ubuntu 24.04 LTS (Noble) publishes `rkdeveloptool` in `universe`: https://packages.ubuntu.com/noble/rkdeveloptool
- Reference RK3288 hardware has distinct **Loader**, **MaskRom**, **Normal** modes; board-specific recovery key procedures described for other boards are **not portable** to MPC One: https://community.t-firefly.com/en/docs/products/motherboard/Firefly-RK3288/bootmode

**Safest next evidence**: confirm a standard data-capable USB-A-to-USB-B connection directly between workstation and MPC One USB-B; run host `udevadm monitor --kernel --subsystem-match=usb` in one terminal while disconnecting/reconnecting **only the cable**, with the MPC left running normally; capture any USB addition/removal events, then `lsusb` and `lsusb -d 2207:`. This does not boot, reflash or alter eMMC. Only once a Rockchip USB device is actually enumerated does `rkdeveloptool ld` add mode-classification evidence. The tool can be installed with `sudo apt install rkdeveloptool` (Noble universe must be enabled if unavailable). **Do not** issue `rkdeveloptool db/wl/ef/rd/ul`, change reboot-mode registers, short an eMMC pin, or assume unrelated RK3288 board recovery-button combinations apply.

**Next hardware research requirement:** document the MPC One's actual USB-B D+/D- and VBUS routing to the RK3288 USB OTG controller, and identify any authorized, non-destructive ROM recovery selection mechanism before attempting to change boot state.

## 23. Host USB diagnostics plus MPC One USB-B / DWC2 board-specific evidence (2026-11-10)

**Owner's second host survey** (`Pasted markdown(9).md`): Ubuntu package installation of `rkdeveloptool` 1.32 succeeded from `resolute/universe` on the workstation. A `udevadm monitor --kernel --subsystem-match=usb` session printed *no USB events* before Ctrl+C. A subsequent `lsusb` again listed only workstation peripherals (no recognizable Akai or Rockchip device), `lsusb -d 2207:` returned no device, and `rkdeveloptool ld` printed **`No devices in rockusb mode found`**. It is not documented whether a cable was present, direct, data-capable, or reseated **while** the monitor ran. Do not infer a disconnected port, absent BootROM, bad USB cable, or disabled recovery circuit from this snapshot alone.

### 23.1 Normal standalone USB inactivity is expected by design

Akai's MPC X/Live/One user guide expressly says that, in **Standalone Mode**, the device is **not communicating with a computer over USB**. Controller Mode binds the USB device to the host. Therefore normal standalone non-enumeration is not an anomaly and does **not** reflect silicon BootROM availability. See Akai's guide: https://cdn.inmusicbrands.com/akai/AKMUWBYS/MPC-User-Guide-v2.7.2.pdf and its FAQ: https://support.akaipro.com/en/support/solutions/articles/69000816149-akai-pro-mpc-one-frequently-asked-questions .

Another public MPC hardware note identifies an Akai MPC One MIDI USB descriptor as `09e8:0046` (this identifier is not guaranteed to enumerate on every firmware or mode). **Do not confuse Akai normal-mode USB IDs with Rockchip `2207:320a`**: https://github.com/TheKikGen/MPC-LiveXplore/blob/master/MPC/Midi-Devices.txt .

### 23.2 Strong MPC One-specific external evidence ties USB-B to 0xFF580000

MPC-LiveXplore issue #55 is *specifically about the MPC One*: https://github.com/TheKikGen/MPC-LiveXplore/issues/55 . The owner reports that entering Controller Mode logs `dwc2 ff580000.usb: bound driver configfs-gadget`, activates `usb-gadget.target`, and creates audio/MIDI USB gadget devices; returning to Standalone Mode disconnects the gadget. The same owner reports enabling the USB-B interface through `UDC=ff580000.usb` in Linux userspace, with live MIDI functioning. This is significant **board-specific software/operational evidence** that the MPC One's host-facing USB-B port routes to the RK3288 DWC2 OTG controller at `0xFF580000`, which is also enabled in the owner-extracted *p3 U-Boot* DTB.

**Caveat:** the public report includes a `/sys/devices/platform/usb-otg-mux/state` state switch (`host`/`peripheral`) and writes to Linux ConfigFS. The board may contain a **physical/logic USB mux** whose power-on default matters during BootROM execution. The existence and exact board wiring/default were **not electrically verified** from the report. An OTG controller usable in stock Linux and U-Boot does not automatically prove ROM-level USB signals reach USB-B at reset. No mux/UDC writes are authorized; avoid copying the community gadget enabling script since it changes runtime device configuration and was reported to cause audio/CPU issues.

### 23.3 Safe next observations

From the workstation: if a suitable data cable is present, capture host USB events on connection/reconnection and explicitly compare the **standalone** vs (only if the user voluntarily opts into changing mode) **Controller** mode. Controller Mode can interrupt active MPC project/runtime sessions, so it is not required at this stage. No host enumeration in normal Standalone should be considered expected.

From an already-established *Ethernet SSH shell on the MPC*, purely read-only inspect: `ls -l /sys/class/udc`, `cat /sys/devices/platform/usb-otg-mux/state` if present, `systemctl show -p ActiveState usb-gadget.target`, `ls -l /sys/kernel/config/usb_gadget` if present, and `journalctl -k -b --no-pager | grep -Ei 'ff580000|dwc2|usb.*otg|gadget' | tail -80`. This can distinguish whether the running Linux kernel instantiates DWC2 and a mux, without binding gadgets or rebooting. Avoid logging identifiers such as `serial#` and embedded OEM authorization strings. Kernel DTB configuration can differ between firmware revisions.

**Current MaskROM readiness:** SoC support is documented, OTG-to-USB-B routing has external MPC-specific evidence, host has the Rockchip enumerator; **safe ROM entry and a board-compatible DDR/USB RAM loader are still not demonstrated**. Independent MaskROM recovery must be proven before boot-critical writes.

## 24. MPC One Ethernet SSH hardware survey proves UDC registered and mux in peripheral role (2026-11-10)

The owner ran the Section 23 diagnostic by SSH over the established MPC Ethernet link, **on the device itself**, rather than on ML-tower. Exact owner-observed properties:

| Observation | MPC One value |
| --- | --- |
| Hostname | `mpc-one` |
| Kernel | `Linux mpc-one 6.6.95-az01-2025-06-27-rt55 ... armv7l GNU/Linux` |
| Device-tree model | `Akai Professional MPC One` |
| UDC class | `/sys/class/udc/ff580000.usb -> ../../devices/platform/ff580000.usb/udc/ff580000.usb` |
| Platform peripheral | `/sys/bus/platform/devices/ff580000.usb` exists |
| USB OTG mux state | **`peripheral`** |
| `usb-gadget.target` | `loaded`, `active`, `active` |
| ConfigFS `/sys/kernel/config/usb_gadget` | Present but **empty** |
| DWC2 init | `dwc2 ff580000.usb: EPs: 10, dedicated fifos, 972 entries in SPRAM` |
| Other DWC2 controller | `ff540000.usb`: `DWC OTG Controller`, registers Linux USB bus 1 (host context) |
| Kernel VBUS | `reg-fixed-voltage usb-otg-regulator: vcc_otg_5v supplying 5000000uV` |

The read-only command's last section `head -60` failed because the MPC BusyBox `head` does not accept that option form (`head: invalid option -- '6'`); this did **not** invalidate the preceding device identity, UDC, mux, ConfigFS, or kernel-log results. Use `sed -n '1,60p'` or BusyBox-compatible `head -n 60` instead.

**Interpretation:** This is board-local evidence, not merely a generic Rockchip source or forum anecdote, that the running Akai Linux exposes DWC2 at `ff580000.usb` as an ARMv7 USB Device Controller and that the platform mux currently reports the **peripheral** role. Both facts strongly support the USB-B-to-OTG research hypothesis. However, software-visible mux role in a fully initialized Linux system does **not** prove the mux's physical reset/power-on state while immutable BootROM runs. No chip-level MaskROM enumeration has occurred.

The ConfigFS gadget root is empty despite `usb-gadget.target` being active. The latter is a systemd orchestration target, **not proof of a bound USB function**. The Linux kernel ConfigFS documentation explicitly requires a configured gadget to be bound to a UDC before it can enumerate on the USB host; see https://www.kernel.org/doc/html/latest/usb/gadget_configfs.html . The absence of a ConfigFS gadget is consistent with earlier normal-Standalone host-side `lsusb` and `rkdeveloptool ld` returning no MPC/Rockchip device. A non-ConfigFS gadget driver is still possible and has not been ruled out by this directory listing alone.

### Next safe ROM/USB investigation (no state changes)

1. From existing SSH, inspect **read-only** `/sys/class/udc/ff580000.usb/{state,function,current_speed,maximum_speed,is_otg,soft_connect}` where available. Distinguish a merely registered UDC from a bound gadget and USB connection state.
2. Observe (but do not edit) the USB mux's `state`, the platform device and driver symlinks, and optionally `systemctl list-dependencies usb-gadget.target`. Do not invoke Linux gadget-binding scripts as a workaround for the ROM question.
3. On a directly connected, data-capable USB-B cable, the host can monitor USB events; if no gadget is bound, absence of host enumeration may be expected. Even a successful Linux gadget test **does not establish** BootROM routing.
4. Determine the hardware mux reset default from MPC board evidence (schematic, safe passive measurement) and seek a non-destructive BootROM entry mechanism **before** assuming silicon ROM USB can reach the USB-B connector.
5. Continue the **parallel** offline Fastboot `bootm` research: authorization-runtime initialization, image policy and available memory. No on-device `boot`, `oem`, `flash`, `erase`, reset-mode or eMMC tests have been authorized or performed.



## 25. Linux gadget unbound and USB OTG mux remains peripheral (2026-11-10)

The owner ran another read-only remote SSH survey **on the actual MPC One** and supplied `mpc-usb-runtime-state.txt`. It confirms:

| Field | Observed |
| --- | --- |
| `hostname`/`uname -m` | `mpc-one` / `armv7l` |
| `/sys/class/udc/ff580000.usb/state` | **`not attached`** |
| `.../function` | **empty** (no gadget function reported) |
| `.../current_speed` | `UNKNOWN` |
| `.../maximum_speed` | `high-speed` |
| `.../is_otg` | `0` |
| `.../soft_connect` | `Permission denied` when read; do not write it |
| `/sys/devices/platform/usb-otg-mux/state` | **`peripheral`** |
| `/sys/devices/platform/usb-otg-mux/driver` | `../../../bus/platform/drivers/of-mux` |
| `/sys/devices/platform/usb-otg-mux/of_node` | live FDT `/usb-otg-mux` |
| `/sys/bus/platform/devices/ff580000.usb/driver` | `../../../bus/platform/drivers/dwc2` |
| `/sys/kernel/config/usb_gadget` | empty |
| `usb-gadget.target` dependencies | target itself; no concrete gadget unit listed |
| Other USB nodes | `ff500000.usb`, `ff520000.usb`, `ff540000.usb`, `ff580000.usb`, `ff770000.syscon:usbphy`, `usb-otg-regulator` |

**Interpretation:** The working Linux kernel registered DWC2 at `ff580000.usb` and is currently set to peripheral routing, but there is **no reported bound gadget function, no USB attachment, and no negotiated speed**. The earlier host-side absence of MPC enumeration is expected in that state and does not indicate hardware failure or prove any MaskROM impossibility. Linux USB gadget drivers must bind before the host can enumerate; cf. https://www.kernel.org/doc/html/latest/driver-api/usb/gadget.html .

**`is_otg=0` should not be used to reject MaskROM.** That Linux gadget capability bit concerns whether an OTG port supports OTG-specific behavior/descriptors, not whether DWC2 works in USB device mode or whether the immutable RK3288 ROM implements recovery. Likewise, the error when **reading** `soft_connect` does not imply controller failure; the ABI documents it as a **control for logical connect/disconnect**, which must not be written in passive research: https://kernel.org/doc/html/v5.19/admin-guide/abi-stable.html .

**Highest-value next evidence:** the running Linux FDT node `/usb-otg-mux` and the exact GPIO/regulator/pinctrl or hardware-mux implementation it references. The sysfs `of-mux` driver name alone does **not** prove an external discrete analog switch; it might select a PHY role or GPIO mux. Even an established Linux role does **not** prove the default electrical path at cold power-on, before ROM code executes. A safe remote audit can read `available_states`, list and hex-dump selected nonsecret properties from `/proc/device-tree/usb-otg-mux` and `/proc/device-tree/usb@ff580000`, inspect `/sys/kernel/debug/gpio` if already mounted, and read driver symlinks. Do **not** change sysfs mux `state`, `soft_connect`, bind a gadget, reset into a new mode or write boot partitions.

The concurrent **Fastboot RAM-boot** investigation remains in progress independently (§§15–21); this USB survey does not alter the bootm CRC and downloaded-image findings or establish the initial permission-gate state.

## 26. MPC One live FDT identifies USB role selector as virtual-mux; physical USB-B path likely direct (2026-11-10)

The owner supplied read-only SSH output from the running MPC One (`mpc-usb-mux-devicetree.txt`). **This requires a correction to prior speculation** that the `usb-otg-mux` might necessarily be a physical board switch.

- `/sys/devices/platform/usb-otg-mux/state`: `peripheral`; `available_states`: `otg host peripheral`.
- Live `/proc/device-tree/usb-otg-mux`: `compatible = "virtual-mux"`, `status="okay"`, `default-state` property, and child nodes `state-host`, `state-otg`, `state-peripheral`. The owner's BusyBox `od` lacks GNU `-t`, so this run **did not read** the `default-state` property contents.
- Live `/proc/device-tree/usb@ff580000`: `compatible="rockchip,rk3288-usb", "rockchip,rk3066-usb", "snps,dwc2"`, `dr_mode="peripheral"`, `status="okay"`. It has a `usb2-phy` reference.
- Live `/proc/device-tree/usb@ff540000`: same DWC2 compatible family, `dr_mode="host"`, `status="okay"`.
- Live `/sys/kernel/debug/gpio` labels `gpio-249 (USB_MUX_CTRL)`, but **no consumer or output drive value is shown** for that line; its name does not establish a physical multiplexer or a dependency for MaskROM. `gpio-12` is separately held high by `usb-otg-regulator`, and `gpio-218` held high by `vcc-host1-regulator`.

An independently published decompilation of the **RK3288 AZ01 ACVA** device tree shows this exact `virtual-mux` with `default-state = "peripheral"`, and three child `fragment@0/__overlay__` nodes updating `dr_mode` to `otg`, `host`, or `peripheral`, respectively: https://deathcamel58.github.io/denon-reverse-engineering/rk3288-az01-acva-dts.html . This is good model-family corroboration, **not proof the live runtime default-state bytes are identical**. A separate MPC One-compatible USB-audio research project reports that **MPC One and One+ connect USB device capability directly to USB-B**, unlike MPC Live II / X / Force where an external/internal hub-routing mux can intervene; reference: https://github.com/jacob-sabella/mpc-addin-usb-audio . Again, electrical routing remains unmeasured.

**Updated interpretation:** the reported `usb-otg-mux` is a virtual *Linux role/configuration selector*, not evidence of a discrete USB data-path switch. It makes external mux reset-state speculation a less compelling blocker for *MPC One specifically*. MaskROM feasibility is still **not proven**, because immutable BootROM may choose a distinct PHY/USB mode and because no safe ROM entry or compatible DDR stage has been validated. No GPIOs, registers, boot partitions or USB gadget bindings were changed.

Next read-only confirmation from remote SSH (BusyBox compatible):

```sh
printf 'Live default-state: '; tr '\\000' '\\n' </proc/device-tree/usb-otg-mux/default-state
for mode in otg host peripheral; do
  f="/proc/device-tree/usb-otg-mux/state-$mode/fragment@0/__overlay__/dr_mode"
  [ -r "$f" ] && { printf '%s overlay dr_mode: ' "$mode"; tr '\\000' '\\n' <"$f"; }
done
```

On the **Fastboot / Ghidra track**, full program-level analysis is *not complete*: the user's imported p3 payload was still at base `0x00000000` rather than linked base `0x00100000`, and most xrefs/decompilation were therefore unavailable. Offline Thumb disassembly has established command dispatch, download/callback flow, bootm CRC call graph, but not end-to-end Linux handoff, default runtime unlock state, safe available DRAM or ROM entry. To finish, import a **separate** correctly based p3 program, analyze functions/xrefs and document the outstanding concrete call graph; do not substitute inferred function names for confirmed dynamic behavior.
