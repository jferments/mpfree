#!/usr/bin/env python3
"""Source-only broad bootchain evidence matrix. Never executes firmware or accesses USB."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import stat
import zipfile

SCHEMA = "mpfree-seven-track-evidence-matrix-v1"
INDEX = "ZIP_INDEX.json"
SOURCE = {
    "boot": "static/wide/storage-env-boot-dtb.json",
    "diff": "static/wide/firmware-3.10-bootchain-v4-compare.json",
    "graph": "static/wide/bootchain-v5-synthesis.json",
    "p1": "static/wide/p1-sram-ghidra.json",
    "p3": "static/wide/p3-uboot-ghidra.json",
    "ledger": "static/wide/evidence-ledger.json",
    "fdt": "static/wide/p1-p3-embedded-fdt-crossstage.json",
    "uboot": "static/v11/v11-ghidra-uboot.json",
    "spl": "static/v11/v11-ghidra-spl.json",
    "updater": "static/v11/v11-ghidra-az01-update.json",
    "library": "static/v11/v11-ghidra-libaz01-prog.json",
    "usb": "usb-protocol-evidence.json",
    "framing": "usb-ff617a-framing.json",
}
PUBLIC = {
    "rk3288_trm": "https://rockchip.fr/Rockchip%20RK3288%20TRM%20V1.0%20Part%201-System%20and%20System%20Control.pdf",
    "boot_magic": "https://github.com/rockchip-linux/u-boot/blob/next-dev/arch/arm/include/asm/arch-rockchip/boot_mode.h",
    "rk_ramboot": "https://lists.u-boot-project.org/pipermail/u-boot/2025-November/603421.html",
    "fastboot": "https://docs.u-boot-project.org/en/latest/android/fastboot.html",
}

def verify_zip(z):
    items = z.namelist()
    if len(items) != len(set(items)):
        raise ValueError("Duplicate ZIP members")
    index = json.loads(z.read(INDEX))
    if index.get("schema") != "mpfree-bootchain-integrated-zip-index-v1":
        raise ValueError("Unsupported ZIP index")
    entries = index["files"]
    if not 1 <= len(entries) <= 250 or set(items) != {x["name"] for x in entries} | {INDEX}:
        raise ValueError("ZIP members mismatch index")
    if sum(z.getinfo(x["name"]).file_size for x in entries) > 384*1024*1024:
        raise ValueError("Uncompressed ZIP too large")
    for x in entries:
        name = x["name"]
        if name.startswith("/") or any(p in ("", ".", "..") for p in name.split("/")):
            raise ValueError("Unsafe ZIP member path")
        blob = z.read(name)
        if len(blob) != x["bytes"] or hashlib.sha256(blob).hexdigest() != x["sha256"]:
            raise ValueError("ZIP member hash mismatch: " + name)
    if z.testzip() is not None:
        raise ValueError("ZIP CRC mismatch")
    return len(entries)

def build(read):
    d = {k: json.loads(read(v)) for k,v in SOURCE.items()}
    b, diff, g, u = d["boot"], d["diff"], d["graph"], d["uboot"]
    env = b["p2"]["boot_related_variables"]
    scripts = diff["boot_script"]
    p1_paths = g["bootloader_stages"]["p1"]["paths"]
    p3_paths = g["bootloader_stages"]["p3"]["paths"]
    functions = u["analyzed_functions"]
    records = []
    def row(track, question, status, evidence, next_step, source):
        records.append({
            "id": "Q%03d" % (len(records)+1), "track":track,
            "question":question, "status":status, "finding":evidence,
            "next_step":next_step, "evidence_source":source})
    confirmed = lambda predicate: "static-supported" if predicate else "unresolved"
    row("ROM","Does RK3288 BootROM have USB OTG fallback?","upstream-supported",
        "RK3288 TRM describes ROM checking ID blocks in NAND/eMMC/SPI/SDMMC and USB OTG loader fallback. No MPC board access proven.",
        "Determine actual MPC OTG port/device-role wiring.",PUBLIC["rk3288_trm"])
    row("ROM","What distinguishes ROM download from RockUSB loader?","upstream-supported",
        "Rockchip U-Boot defines BOOT_BROM_DOWNLOAD=0xEF08A53C; BOOT_LOADER=0x5242C301.",
        "Verify this board's U-Boot implements ROM reboot and available USB gadget.",PUBLIC["boot_magic"])
    row("ROM","Is physical MaskROM enumeration verified?","hardware-gated",
        "No confirmed BootROM USB enumeration from the real MPC One.",
        "Collect read-only host USB VID/PID/interface enumeration in a safe state.","physical MPC")
    row("ROM","Can newer Rockchip 0x471/0x472 RAM boot be assumed on RK3288?","unresolved",
        "No. Upstream RAM-boot proposals target other Rockchip boards; this MPC's ROM protocol and DDR payload remain unverified.",
        "Trace RK3288-specific ROM implementation and DRAM entry.",PUBLIC["rk_ramboot"])
    row("ROM","Is BOOT_BROM_DOWNLOAD literal present in captured stages?","static-supported",
        "Current P1/P3 raw literal scanner reports no 0xEF08A53C hit; a missing literal does not exclude computed values.",
        "Trace ARM boot-mode register write sites.","static/wide/p1-sram-ghidra.json + p3-uboot-ghidra.json")
    row("ROM","Is ROM security/eFuse state determined?","hardware-gated",
        "Userland crypto functions and stage bytes cannot reveal silicon security policy.",
        "Read security fuse state with non-mutating authorized diagnostics.","static/wide/evidence-ledger.json")
    spl_sources=d["spl"].get("direct_boot_probe",{}).get(
        "original_image_pointers",{})
    spl_entry=spl_sources.get("candidate_stage_entry_handoff",{}).get(
        "original_word")
    spl_initial=d["spl"].get("analyzed_functions",{}).get(
        "0xff705208",{}).get("pseudocode","")
    row("SPL","Does SRAM initialization make an indirect call to the boot selector?",
        confirmed(spl_entry=="0xff705351" and
                  "(*DAT_ff705274)(" in spl_initial),
        "Yes: original SPL slot 0xFF705274 is Thumb pointer 0xFF705351 to selector 0xFF705350, called by 0xFF705208. This closes the missing graph edge statically, not ROM acceptance.",
        "Inspect full entry interworking and physical ROM acceptance independently.",
        "static/v11/v11-ghidra-spl.json")
    row("SPL","Is the second context-based function-pointer handoff target established?",
        "unresolved",
        "Original callback slot 0xFF70A250 is 0x00000000 in saved SPL. The 0xFF705310 indirect caller needs runtime initialization or a different control path; the executed handoff target remains unknown.",
        "Trace all writers to context 0xFF70A244+0xC and the later backend initializer; zero original bytes do not prove a null runtime call.",
        "static/v11/v11-ghidra-spl.json")
    spl_probe=d["spl"].get("direct_boot_probe",{}).get(
        "extended_stage_evidence",{})
    spl_tables=spl_probe.get("opaque_media_table_words",{})
    source_labels={x.get("at","").lower():x.get("text") for x in
                   d["p1"].get("strings",[])}
    map_entries=spl_tables.get("device_mapping",{}).get("entries",[])
    order_entries=spl_tables.get("device_order",{}).get("entries",[])
    labels=[source_labels.get(map_entries[j]["word"].lower())
            for j in (1,3,5)] if len(map_entries)==8 else []
    row("SPL","Does the original five-slot boot priority table contain live priorities?",
        "static-supported" if (len(order_entries)==5 and all(
            x.get("word")=="0xdeadbeef" for x in order_entries))
        else "unresolved",
        "No: all five saved table slots are 0xDEADBEEF; the selector invokes 0xFF705342 which writes FUN_ff704120's selected boot-device value into the first slot before iteration.",
        "Trace runtime order initialization and actual boot-device source; do not infer eMMC/SD order from placeholders.",
        "static/v11/v11-ghidra-spl.json")
    row("SPL","Can the first three SPL media ID labels be identified?",
        confirmed(labels==["MMC1","MMC2","MMC2_2"]),
        "Yes: mapping IDs 1/2/3 point to original strings MMC1, MMC2, MMC2_2. The sentinel fourth pair remains opaque; label is not physical port proof.",
        "Map names through MMC driver and actual RK3288 controller addresses.",
        "static/v11/v11-ghidra-spl.json + static/wide/p1-sram-ghidra.json")
    row("SPL","Do the backup P1 offsets and size identify stage location?",confirmed(b["p1"]["lba"]==64 and b["p1"]["bytes"]==65536),
        "The first-stage partition starts at LBA 64 and spans 64KiB in the owner's backup.",
        "Confirm an independent ROM loader can accept an owner-built SRAM payload.","static/wide/storage-env-boot-dtb.json")
    row("SPL","Does SPL initializer 0xFF705208 indirectly reach boot selector?",
        confirmed(spl_entry=="0xff705351" and
                  "(*DAT_ff705274)(" in spl_initial),
        "Yes: 0xFF705208 calls via pointer 0xFF705274 into Thumb 0xFF705350. This is a local SPL graph edge, NOT proof ROM reached initializer.",
        "Continue tracing immutable-ROM handoff and second-stage runtime callback initialization.",
        "static/v11/v11-ghidra-spl.json")
    row("SPL","Can SPL selection reach media load?",
        confirmed(p1_paths["boot_loop_to_media_load"]["direct_call_path_found"]),
        "Boot selector 0xFF705350 directly calls loader 0xFF7054A4.",
        "Determine media success/error and validation branches.","static/wide/bootchain-v5-synthesis.json")
    row("SPL","Can SPL reach its indirect second-stage handoff?",
        confirmed(p1_paths["boot_loop_to_indirect_handoff"]["direct_call_path_found"]),
        "Boot selector reaches indirect handoff at 0xFF705310.",
        "Resolve target address, memory bounds and loaded bytes.","static/wide/bootchain-v5-synthesis.json")
    row("SPL","Are DDR parameters cross-stage consistent?","static-supported",
        "Prior evidence identifies three matching P1/P3 DDR parameter arrays.",
        "Study DDR setup and design RAM-only tests after recovery is demonstrated.","static/wide/p1-p3-embedded-fdt-crossstage.json")
    row("SPL","Has owner-built SPL executed or initialized DRAM?","hardware-gated",
        "No owner-controlled first-stage code has been demonstrated on the physical MPC.",
        "Verify independent nonpersistent loader acceptance first.","physical MPC")
    row("U-Boot","Is P2 environment CRC valid?",confirmed(bool(b["p2"]["crc_valid_for_standard_layout"])),
        "CRC32 matches saved U-Boot environment format; not evidence which environment is active.",
        "Read actual U-Boot environment without changes.","static/wide/storage-env-boot-dtb.json")
    comparisons=b["compiled_uboot_environment"]["saved_comparison"]
    row("U-Boot","Do embedded defaults match saved P2 settings?",
        confirmed(all(v.get("same_as_saved") for v in comparisons.values())),
        "%d/%d candidate boot variables match." % (sum(v.get("same_as_saved",False) for v in comparisons.values()),len(comparisons)),
        "Establish runtime environment selection and override precedence.","static/wide/storage-env-boot-dtb.json")
    # Embedded U-Boot FDT uses aliases mmc0->ff0f0000 and
    # mmc1->ff0c0000. The latter controller is explicitly disabled.
    fdt_text="\n".join(d["fdt"]["uboot_stage_fdt"][0]["selected_dts_lines"])
    mmc0_emmc = ('mmc0 = "/dwmmc@ff0f0000";' in fdt_text and
                 'mmc1 = "/dwmmc@ff0c0000";' in fdt_text)
    mmc1_chunk = fdt_text.split("dwmmc@ff0c0000 {",1)[1].split("};",1)[0] if "dwmmc@ff0c0000 {" in fdt_text else ""
    mmc0_chunk = fdt_text.split("dwmmc@ff0f0000 {",1)[1].split("};",1)[0] if "dwmmc@ff0f0000 {" in fdt_text else ""
    def disabled_controller(address):
        node = "dwmmc@" + address + " {"
        if node not in fdt_text:
            return False
        chunk = fdt_text.split(node, 1)[1].split("};", 1)[0]
        return 'status = "disabled";' in chunk
    other_mmc_disabled = all(disabled_controller(x) for x in (
        "ff0c0000", "ff0d0000", "ff0e0000"))
    second_mmc_disabled = ('status = "disabled";' in mmc1_chunk)
    first_mmc_emmc = ('status = "okay";' in mmc0_chunk and
                      'non-removable;' in mmc0_chunk and
                      'bus-width = <0x08>;' in mmc0_chunk)
    row("U-Boot","Which MMC devices are tried in U-Boot order?","static-supported",
        "Configured boot_targets=%s with bootdelay=%s; embedded U-Boot FDT aliases mmc0=eMMC-controller and mmc1=SDMMC-controller." %
        (env["boot_targets"].strip(),env["bootdelay"]),
        "Verify runtime probing and board-level removable storage wiring.","static/wide/storage-env-boot-dtb.json + p1-p3-embedded-fdt-crossstage.json")
    row("U-Boot","Does embedded U-Boot FDT enable its SDMMC boot target?",
        confirmed(mmc0_emmc and first_mmc_emmc and second_mmc_disabled and other_mmc_disabled),
        "Embedded U-Boot FDT maps mmc0 to active non-removable 8-bit eMMC at ff0f0000; all three other MMC nodes ff0c0000 (mmc1), ff0d0000 (mmc2), ff0e0000 (mmc3) are disabled. Runtime re-enable is unproven.",
        "Determine if U-Boot dynamically enables SDMMC or reads a different runtime FDT before relying on SD fallback.",
        "static/wide/p1-p3-embedded-fdt-crossstage.json")
    row("U-Boot","Is there a stock fallback to Fastboot?",
        confirmed("fastboot 0" in env["az01_recovercmd"]),
        "az01_bootcmd branches on dorecovery into az01_recovercmd/fastboot 0.",
        "Trace reboot-flag to runtime recovery behavior.","static/wide/storage-env-boot-dtb.json")
    reboot_code=functions.get("0x00100fe4",{}).get("pseudocode","")
    reboot_modes=all(part in reboot_code for part in (
        "(*DAT_0010114c & 0xffffff00) == DAT_00101150",
        "if (uVar4 == 1)", "if (uVar4 == 9) goto LAB_00101096;",
        "*DAT_00101154 = 1;", "*puVar1 = 0;"))
    row("U-Boot","Does this exact U-Boot handle loader and Fastboot reboot-mode suffixes?",
        confirmed(reboot_modes),
        "Original U-Boot 0x100FE4 checks the high-byte prefix, then low-byte values 1/9, sets a recovery variable and clears the consumed word.",
        "Resolve exact mode register, prefix and runtime path; no MaskROM claim.",
        "static/v11/v11-ghidra-uboot.json")
    threshold=all(part in reboot_code for part in (
        "FUN_001147f0(10,0,&local_20)",
        "FUN_00115f30(local_20,1,local_1c)",
        "if (local_1c[0] < 100)"))
    adc_messages={
        "Recovery button ADC not found":"001010aa",
        "Failed to start recovery button ADC channel":"001010f0",
        "Failed to read recovery button ADC channel":"00101100",
    }
    adc_xrefs=all(any(
        row.get("text")==message and
        any(ref.get("from","").lower()==callsite
            for ref in row.get("xrefs",[]))
        for row in d["p3"].get("strings",[]))
        for message,callsite in adc_messages.items())
    arm={str(x.get("address","")).lower():x.get("text")
         for x in functions.get("0x00100fe4",{}).get(
             "machine_instruction_evidence",[])}
    adc_conditional=(arm.get("0010110c")=="cmp r3,#0x63" and
                     arm.get("0010110e")=="ittt ls" and
                     arm.get("00101114")=="strb.ls r2,[r3,#0x0]")
    row("U-Boot","Is the below-100 recovery trigger the stock recovery-button ADC?",
        confirmed(threshold and adc_xrefs and adc_conditional),
        "Yes: three diagnostics explicitly identify the recovery-button ADC; original ARM compares unsigned <=99 then stores 1 into the recovery flag. Not a boot-counter observation.",
        "Resolve the recovery flag shared pointer and external physical button-to-ADC mapping.",
        "static/v11/v11-ghidra-uboot.json + static/wide/p3-uboot-ghidra.json")
    row("U-Boot","Does Fastboot getvar use the same ADC class with a separate channel?",
        confirmed(all(word in functions.get("0x00100ef0",{}).get("pseudocode","")
                      for word in (
                          "FUN_001147f0(10,0,&local_24)",
                          "FUN_00115ef4(local_24,0)",
                          "FUN_00115f30(local_24,0,&local_20)"))),
        "Yes: the getvar fallback path selects device class 10, sequence 0, ADC channel 0; recovery-button path uses channel 1.",
        "Decode getvar request key and status text, but do not interpret channel-0 voltage without source evidence.",
        "static/v11/v11-ghidra-uboot.json")
    row("U-Boot","Does the recovery flag reach dorecovery and Fastboot?",
        confirmed(d["uboot"].get("direct_boot_probe",{}).get(
            "recovery_adc_to_environment_pointer_equal") is True),
        "Original ADC writer and board environment reader pointers both equal 0x001462FC; board setup exports dorecovery and saved environment selects fastboot 0.",
        "Identify physical button and confirm ADC threshold read-only on an owner-authorized live device.",
        "static/v11/v11-ghidra-uboot.json + static/wide/storage-env-boot-dtb.json")
    row("U-Boot","Are extlinux and external script fallback configured?","static-supported",
        "Saved U-Boot environment scans extlinux.conf and boot.scr.uimg/boot.scr in mmc targets.",
        "Check which device can actually provide a verified reversible boot path.","static/wide/storage-env-boot-dtb.json")
    uboot_getvar=d["uboot"].get("direct_boot_probe",{}).get(
        "reboot_mode_original_literal_slots",{}).get(
        "getvar_adc_key_string",{}).get("printable_string")
    row("U-Boot","Does Fastboot getvar:power-source query SARADC channel 0?",
        confirmed(uboot_getvar=="power-source" and
                  "FUN_00115ef4(local_24,0)" in
                  d["uboot"].get("analyzed_functions",{}).get(
                      "0x00100ef0",{}).get("pseudocode","")),
        "Yes: request key power-source is read from the original literal pool; the getvar callback starts channel 0 and scales its ADC reading/reference voltage into a categorical response.",
        "Decode the actual scale, boundaries and four response labels; no physical voltage has been measured.",
        "static/v11/v11-ghidra-uboot.json")
    pwr=d["uboot"].get("direct_boot_probe",{}).get(
        "reboot_mode_original_literal_slots",{})
    def pw_word(key):
        val=pwr.get(key,{}).get("value")
        return int(val,16) if isinstance(val,str) else None
    pwr_bounds=[pw_word("getvar_power_source_threshold_"+str(i))
                for i in (1,2,3)]
    pwr_labels=[pwr.get("getvar_power_source_label_"+str(i)+"_string",{}).get(
        "printable_string") for i in (1,2,3,4)]
    row("U-Boot","Are exact Fastboot power-source responses and thresholds known?",
        confirmed(pwr_bounds==[624999,1089999,1249999] and
                  pwr_labels==["invalid","battery","ac","invalid-ac"] and
                  pw_word("getvar_power_source_scale")==0x10624dd3),
        "Yes: original literals bound the computed result to invalid <=624999, battery <=1089999, ac <=1249999, else invalid-ac; fixed-point multiplier is ~1/1000. Physical units unknown.",
        "Verify exact ARM signed/unsigned arithmetic and ADC reference units, not live voltages.",
        "static/v11/v11-ghidra-uboot.json")
    row("U-Boot","Does Fastboot RAM boot reach bootm?",
        confirmed(p3_paths["fastboot_ram_boot_to_bootm"]["direct_call_path_found"]),
        "Direct path from 0x1226B4 to 0x101744 and image loader 0x10A6A4.",
        "Inspect image format, RAM download buffer boundaries and execution permissions.","static/wide/bootchain-v5-synthesis.json")
    writes=all("mpf_fastboot_permission" in functions[a].get("pseudocode","") for a in ("0x00122c5c","0x00122bf8"))
    row("U-Boot","Are Fastboot flash and erase write-permission gated?",confirmed(writes),
        "Both static flash and erase handlers call mpf_fastboot_permission.",
        "Trace permission global writes and OEM unlock candidate without running commands.","static/v11/v11-ghidra-uboot.json")
    row("U-Boot","Is Fastboot RAM boot governed by the same write gate?",
        "static-partial",
        "The boot callback contains no local permission helper; normal image delivery through download: is explicitly permission gated. Alternative RAM delivery routes and live state remain unproven.",
        "Trace initialized download buffer and accepted bootm image types, avoiding physical writes.",
        "static/v11/v11-ghidra-uboot.json")
    row("U-Boot","Is Fastboot unlocked on the physical MPC?","hardware-gated",
        "Ghidra shows a global permission check but not live initialization.",
        "Use read-only runtime inquiry if reachable.","static/v11/v11-ghidra-uboot.json")
    row("U-Boot","Are P3 legacy uImage CRCs intact (not authenticated)?",
        confirmed(b["p3"]["header_crc32_valid"] and b["p3"]["payload_crc32_valid"]),
        "Header/payload CRC32 are valid but are not cryptographic authentication.",
        "Separate FIT signature checks from legacy checksum validation.","static/wide/storage-env-boot-dtb.json")
    row("U-Boot","Does image classification reach FIT verifier?",
        confirmed(p3_paths["image_classifier_to_fit_verify"]["direct_call_path_found"]),
        "Static classifier-to-verifier path exists; trust roots and enforcement remain unknown.",
        "Trace verifier branches and keys.","static/wide/bootchain-v5-synthesis.json")
    row("Memory","Is RAM boot memory placement safe?","unresolved",
        "Env uses FDT 0x01F00000, kernel 0x02000000 and ramdisk 0x04000000; relocation/stack/download footprint not measured.",
        "Calculate image boundaries and verify physical runtime relocation before using boot commands.","static/wide/storage-env-boot-dtb.json")
    row("Memory","Do static DDR parameter matches establish safe memory ownership?","unresolved",
        "Matching DDR parameters do not establish U-Boot relocation, stack, cache, framebuffer or USB download footprints.",
        "Measure and compare stage relocation, kernel expansion and FDT reserve ranges.","static/wide/p1-p3-embedded-fdt-crossstage.json")
    row("Memory","Does generic ARM QEMU validate RK3288 memory registers?","unresolved",
        "No: generic ARM virt uses different hardware and cannot exercise RK3288 ROM/DDR/USB MMIO.",
        "Keep QEMU userspace observations separate from physical bring-up evidence.","session-summary.json")
    row("Update","Did 3.10 alter executable U-Boot boot scripts?",
        confirmed(scripts["normalized_commands_equal"]),
        "Normalized 3.6 and 3.10 boot script commands match.",
        "Trace userspace/kernel/device-tree differences.","static/wide/firmware-3.10-bootchain-v4-compare.json")
    row("Update","Does official boot-script SHA describe the complete legacy uImage?",
        "static-supported",
        "No: 3.10 comparison stores raw script payload SHA under sha256, the 64-byte header SHA separately, and the 535-byte total length in header.image_bytes. Comparing this sha256 to guest complete-image hash is invalid.",
        "Verify original script header and payload independently against any guest copy, including terminating newline.",
        "static/wide/firmware-3.10-bootchain-v4-compare.json")
    names=("/boot/rk3288-az01-acva.dtb","/boot/rk3288-az01-acva-c.dtb")
    changes=[diff["board_dtbs"][name]["boot_node_diff"]["changed_boot_nodes"] for name in names]
    aliases=sum(any("mode-update = <0x5242c301>" in "\n".join(x.get("after",[]))
                    and "mode-loader = <0x5242c301>" in "\n".join(x.get("after",[]))
                    for x in diffnodes.values()) for diffnodes in changes)
    row("Update","Does new 3.10 mode-update equal loader mode?",
        confirmed(aliases==2),
        "%d/2 MPC One DTBs alias mode-update to loader 0x5242C301, distinct from BootROM 0xEF08A53C." % aliases,
        "Trace updater-triggered reboot into the mode register and subsequent loader path.","static/wide/firmware-3.10-bootchain-v4-compare.json")
    sig = diff["elfs"]["/usr/bin/az01-bootloader-sig"]
    row("Update","Does 3.10 add a userspace bootloader signature tool?",
        confirmed(not sig["factory"]["present"] and sig["firmware310"]["present"]),
        "az01-bootloader-sig appears in 3.10 but not the 3.6 backup.",
        "Find invocation sites and clarify whether it verifies update contents or boot-time code.","static/wide/firmware-3.10-bootchain-v4-compare.json")
    row("Update","Does userspace SHA-256 prove ROM secure boot?","static-supported",
        "No. The new utility's SHA-256 comparison is distinct from BootROM enforcement.",
        "Trace trust assumptions in both layers separately.","static/wide/bootchain-v5-synthesis.json")
    trust=diff["services"]["/usr/lib/systemd/system/az0x-loadpin-trustpoints.service"]["firmware310"]
    row("Update","Is a 3.10 early verity LoadPin service present?",confirmed(trust["present"]),
        "New oneshot trustpoint setup precedes local-fs-pre.target.",
        "Trace its inputs and whether dm-verity is actually configured.","static/wide/firmware-3.10-bootchain-v4-compare.json")
    row("Update","Is a separate ramfs-update SquashFS included?",
        confirmed(diff["ramfs_updater"]["present"]),
        "Updater ramfs inventory is available but neither its program nor writes were run.",
        "Map verified update preparation, target selection and rollback behavior.","static/wide/firmware-3.10-bootchain-v4-compare.json")
    updater_stub=d["updater"].get("analyzed_functions",{})
    stub_bad=all("halt_baddata" in updater_stub.get(address,{}).get(
        "pseudocode","") for address in (
            "0x00014028","0x00014040","0x00014070"))
    row("Update","Do three recognized updater trampoline addresses prove library implementations?",
        "static-supported" if stub_bad else "unresolved",
        "The invalid-instruction addresses are updater linkage stubs; verified ELF32 ARM relocations pair all three imports with defined libaz01-prog exports. Actual runtime binding and function side effects remain unproven.",
        "Trace the exported library implementations, cryptographic checks, and indirect backend callbacks.",
        "static/v11/v11-ghidra-az01-update.json + elf-relocation-crosscheck.json")
    library=d["library"].get("analyzed_functions",{})
    download_code=library.get("0x0001b9bc",{}).get("pseudocode","")
    write_code=library.get("0x0001e194",{}).get("pseudocode","")
    signature_code=library.get("0x0001f094",{}).get("pseudocode","")
    row("Update","Does the updater download path attempt verification before applying?",
        confirmed(all(x in download_code for x in (
            "az01_image_set_onboard_key_lookup(local_1064)",
            "az01_verify_image(local_1064,0)",
            "az01_apply_image("))),
        "libaz01-prog downloaded-image path selects onboard key lookup and checks verification before entering apply_image. No vendor execution or signature policy guarantee.",
        "Trace verification callback return propagation and which image families choose this path.",
        "static/v11/v11-ghidra-libaz01-prog.json")
    row("Update","Does library partition write orchestrate verification before preparing storage?",
        confirmed("az01_verify_image(param_1,2)" in write_code and
                  "az01_prepare_to_write(" in write_code and
                  "az01_image_is_compatible(" in write_code),
        "az01_write_image invokes verification first, then flag-conditional device/partition compatibility, then prepare_to_write and partition enumeration. Cached verification state changes semantics.",
        "Trace all flag states, callback error propagation, and commit behavior.",
        "static/v11/v11-ghidra-libaz01-prog.json")
    row("Update","Does libaz01-prog implement an OpenSSL SHA-256 signature verification helper?",
        confirmed(all(x in signature_code for x in (
            "EVP_sha256()","EVP_DigestVerifyInit(",
            "EVP_DigestVerify(ctx,param_4,0x100,param_2,param_3)"))),
        "Yes: EVP_DigestVerify with SHA-256 and 256-byte signature argument. Key algorithm, active keys, and universal enforcement have not been established.",
        "Trace get_public_key/image signature records and failure-state policy before claiming secure updates.",
        "static/v11/v11-ghidra-libaz01-prog.json")
    row("Update","Are file, network and USB updater modes exposed?","static-supported",
        "Updater executable strings and symbols expose file/net/USB update paths; execution path still unproven.",
        "Trace CLI entry→verification→image apply→reboot in Ghidra.","static/wide/firmware-3.10-bootchain-v4-compare.json")
    usbs=d["usb"]["verified_backend_interfaces"]
    usb_slots={}
    for x in d["usb"]["verified_backend_interfaces"]:
        desc=x.get("usb_descriptor",{})
        if all(desc.get(k) for k in ("class","subclass","protocol")):
            label="/".join(desc[k].lower() for k in
                           ("class","subclass","protocol"))
            usb_slots[label]={
                row["offset"]:row.get("resolved_function")
                for row in x.get("backend_callback_evidence",[])
                if row.get("offset")}
    row("USB","Does vendor write API route through backend write/commit slots?",
        confirmed(all(key in d["library"]["analyzed_functions"].get(addr,{}).get(
            "pseudocode","") for addr,key in (
                ("0x00014b98","+ 0x18"),
                ("0x00014c50","+ 0x1c"),
                ("0x00014ac8","+ 0x2c")))),
        "Yes: generic libaz01-prog API dereferences backend-function slots for bootloader write (+0x18), partition write (+0x1C), and bootloader commit (+0x2C). Actual selection and USB execution unproven.",
        "Match every slot against original backend table and keep null callback semantics distinct from hardware capability.",
        "static/v11/v11-ghidra-libaz01-prog.json")
    row("USB","Do Fastboot, proprietary FF/61/7A, and RockUSB have identical storage callbacks?",
        confirmed(len(usb_slots)==3 and
            usb_slots.get("0xff/0x42/0x03",{}).get("+0x20") is not None and
            usb_slots.get("0xff/0x42/0x03",{}).get("+0x2c") is not None and
            usb_slots.get("0xff/0x61/0x7a",{}).get("+0x20") is not None and
            usb_slots.get("0xff/0x61/0x7a",{}).get("+0x2c") is None and
            usb_slots.get("0xff/0x06/0x05",{}).get("+0x20") is None and
            usb_slots.get("0xff/0x06/0x05",{}).get("+0x2c") is None),
        "No: original Fastboot table has stream+commit callbacks; FF/61/7A has stream but no commit; RockUSB-like table has neither. All have a partition-write callback.",
        "Trace callback return/error contracts per transport; null slots do not exclude an alternate write protocol.",
        "usb-protocol-evidence.json")
    row("USB","How many updater USB interface backends are selected?",
        confirmed(len(usbs)==3),
        "Three interfaces FF/42/03, FF/61/7A and FF/06/05 are selected.",
        "Map runtime device enumeration to one selected backend.","usb-protocol-evidence.json")
    row("USB","Are any interface families recognized?","upstream-supported",
        "FF/42/03 is Fastboot-compatible and FF/06/05 RockUSB-compatible. This does not identify physical ROM state.",
        "Compare exact wire behavior against each upstream implementation.","usb-protocol-evidence.json")
    row("USB","Is FF/61/7A a framed request-response protocol?","static-supported",
        "Static 24-byte header, request ID, correlation, flags, declared length and error handling found.",
        "Test a fake USB endpoint in isolation.","usb-ff617a-framing.json")
    row("USB","Are 0x5AA5 trace-label constants a mandatory receive allowlist?","static-supported",
        "No. Incoming identifier is compared against the outstanding request; six values select diagnostic labels.",
        "Keep conservative MPFree policy separate from vendor behavioral model.","static/v11/v11-ghidra-libaz01-prog.json")
    row("USB","Does the host updater use async USB URBs?","static-supported",
        "USBDEVFS_SUBMITURB/REAPURBNDELAY and poll are present in the updater host transport.",
        "Model short reads, retries and error propagation.","usb-ff617a-framing.json")
    row("USB","Have the real vendor USB functions been executed in QEMU?","unresolved",
        "The current QEMU run only inspects vendor libraries and tests independent synthetic models.",
        "Require a no-passthrough hardened VM and deny-by-default virtual transport.","session-summary.json")
    row("Recovery","Is a valid source eMMC backup available?","static-supported",
        "Owner backup authenticated P1/P2/P3 source bytes. Its existence is not independent restoration.",
        "Establish a real ROM-mode full-storage readback channel.","static/wide/manifest.json")
    row("Recovery","Is independent MaskROM restore verified?","hardware-gated",
        "No physical MaskROM entry, owner-built SRAM loader, or independent full eMMC recovery yet.",
        "Do not alter boot partitions until every recovery gate has proof.","physical MPC")
    # Bounded, source-only directed decompiler footprint across four roles.
    # No proprietary pseudocode or raw instruction text is placed in the
    # public repository; these derived details remain private in the ZIP.
    focus={
        "spl":("0xff704004","0xff705316","0xff705350","0xff7054a4","0xff705310"),
        "uboot":("0x00100fe4","0x00101744","0x0010f388","0x001226b4",
                 "0x00122834","0x00122854","0x00122bf8","0x00122c5c"),
        "updater":("0x0001091c","0x00010be4"),
        "library":("0x0001a314","0x0001a674","0x0001d644"),
    }
    directed=[]
    for role, addresses in focus.items():
        funcs=d[role].get("analyzed_functions",{})
        for address in addresses:
            f=funcs.get(address,{})
            directed.append({
                "role":role,"address":address,
                "recognized":bool(f),
                "decompiled":bool(f.get("pseudocode")),
                "captured_machine_instructions":len(f.get("machine_instruction_evidence",[])),
                "direct_callees":len(f.get("graph",{}).get("callees",[])),
                "computed_call_sites":len(f.get("graph",{}).get("computed",[])),
                "instruction_capture_limited":bool(f.get("instruction_capture_limited",False)),
            })
    counts={x:sum(r["status"]==x for r in records) for x in
            ("static-supported","upstream-supported","unresolved","hardware-gated")}
    return {
        "schema":SCHEMA,"questions":records,"counts":counts,
        "focused_static_functions":directed,"sources":SOURCE,
        "public_references":PUBLIC,"physical_device_access":False,
        "vendor_code_executed":False,
        "next_round_scope":[
            "USB: isolate transport error and mock device behavior.",
            "ROM: verify BootROM-vs-loader request path and RK3288 SRAM limits.",
            "SPL: identify entry/interworking and indirect handoff target.",
            "U-Boot: map Fastboot permission writers, RAM boot and FIT policy.",
            "Update: trace 3.10 verification, ramfs, reboot flag and LoadPin.",
            "Memory/recovery: investigate MMC topology, RAM bounds and reversible boot.",
        ],
    }

def markdown(report):
    lines=["# MPFree multi-track research question matrix","",
           "Offline owner-static facts, upstream references and hardware gates are separate.",
           "No original vendor execution or physical device access.","",
           "Questions: %d" % len(report["questions"]),
           "Categories: "+json.dumps(report["counts"],sort_keys=True),""]
    for track in dict.fromkeys(q["track"] for q in report["questions"]):
        lines.extend(["","## "+track,""])
        for q in report["questions"]:
            if q["track"]!=track:
                continue
            lines.extend([
                "### "+q["id"]+" "+q["question"],
                "Status: "+q["status"],"",
                q["finding"],"",
                "Evidence: "+q["evidence_source"],"",
                "Next: "+q["next_step"],"",
            ])
    lines.extend(["","## Directed Ghidra evidence inventory","",
                  "Read-only instruction counts, no copied pseudocode.",""])
    for f in report["focused_static_functions"]:
        lines.append("- "+f["role"]+" "+f["address"]+
                     ": decompiled="+str(f["decompiled"])+
                     ", ARM instructions="+str(f["captured_machine_instructions"])+
                     ", computed calls="+str(f["computed_call_sites"]))
    lines.extend(["","## Next run breadth guarantees"])
    lines.extend("- "+v for v in report["next_round_scope"])
    return "\n".join(lines)+"\n"

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--input-zip",required=True,type=Path)
    ap.add_argument("--output-zip",required=True,type=Path)
    a=ap.parse_args()
    if a.input_zip.is_symlink() or not a.input_zip.is_file() or not stat.S_ISREG(a.input_zip.lstat().st_mode):
        ap.error("Input must be an existing regular ZIP")
    if a.output_zip.exists() or a.output_zip.is_symlink():
        ap.error("Refusing existing output path")
    with zipfile.ZipFile(a.input_zip) as zin:
        verified=verify_zip(zin)
        doc=build(zin.read)
    doc["indexed_input_members_verified"]=verified
    artifacts={
        "wide-research-matrix.json":(json.dumps(doc,indent=2)+"\n").encode(),
        "wide-research-matrix.md":markdown(doc).encode(),
    }
    index={k:{"bytes":len(v),"sha256":hashlib.sha256(v).hexdigest()} for k,v in artifacts.items()}
    artifacts["INDEX.json"]=(json.dumps(index,indent=2)+"\n").encode()
    a.output_zip.parent.mkdir(parents=True,exist_ok=True)
    with zipfile.ZipFile(a.output_zip,"x",compression=zipfile.ZIP_DEFLATED) as z:
        for k,v in artifacts.items():
            z.writestr(k,v)
    print("WIDE_RESEARCH_MATRIX_ZIP:",a.output_zip)
    print("QUESTIONS:",len(doc["questions"]),"COUNTS:",json.dumps(doc["counts"],sort_keys=True))

if __name__=="__main__":
    main()
