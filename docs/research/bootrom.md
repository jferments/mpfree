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
