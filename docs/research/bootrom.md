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
