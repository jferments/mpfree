#!/usr/bin/env python3
"""MPFree read-only seven-topic direct-boot evidence synthesis.

Requires SHA-verified private reports from owner's Ghidra/QEMU archive;
never opens USB/eMMC or runs a vendor executable. Public repo contains
this analyzer only, not firmware or Ghidra pseudocode.
"""
from __future__ import annotations
import argparse, hashlib, json, stat, zipfile
from pathlib import Path

SCHEMA="mpfree-direct-boot-focus-v1"
SOURCE={
    "u":"static/v11/v11-ghidra-uboot.json",
    "p3":"static/wide/p3-uboot-ghidra.json",
    "boot":"static/wide/firmware-3.10-bootchain-v4-compare.json",
    "env":"static/wide/storage-env-boot-dtb.json",
    "fdt":"static/wide/p1-p3-embedded-fdt-crossstage.json",
    "spl":"static/v11/v11-ghidra-spl.json",
    "update":"static/v11/v11-ghidra-az01-update.json",
}
UBOOT_SHA="b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
def build(read):
    d={key:json.loads(read(path)) for key,path in SOURCE.items()}
    u=d["u"]
    if u.get("role")!="uboot" or u.get("source_sha256_prechecked_by_launcher")!=UBOOT_SHA:
        raise ValueError("Wrong original P3 U-Boot ELF image or Ghidra report")
    funcs=u["analyzed_functions"]
    def body(where):return funcs.get(where,{}).get("pseudocode","")
    oem=body("0x00122854")
    permission=body("0x00122834")
    glob=d["p3"].get("globals",{}).get("0x0014d690",{})
    refs={str(r.get("from","")).lower():str(r.get("type","")).upper()
          for r in glob.get("references",[])}
    env=d["env"]["p2"]["boot_related_variables"]
    # Address distances are NOT a verified memory map: they are only
    # candidate overlap thresholds if regions are simultaneously live.
    load_base=int(env["loadaddr"],0)
    nominal_memory_gaps={}
    for name in ("fdt_addr_r","kernel_addr_r","ramdisk_addr_r"):
        target=int(env[name],0)
        distance=target-load_base
        nominal_memory_gaps[name]={
            "address": "0x%08x" % target,
            "from_fastboot_load_base_bytes": distance,
            "from_fastboot_load_base_mib": round(distance/(1024.0*1024),6),
            "non_overlapping_conclusion_valid": False,
        }
    boot=d["boot"]["boot_script"]
    normal=boot["firmware310"]["commands"]
    fdt="\n".join(d["fdt"]["uboot_stage_fdt"][0]["selected_dts_lines"])
    checks={
        "token_prefix_comparison_37_bytes":
            "FUN_0012dc78(DAT_00122924,iVar3,0x25)" in oem,
        "oem_grants_permission_pointer":
            "*DAT_00122928 = 1" in oem,
        "permission_helper_reads_pointer":"*DAT_0012284c" in permission,
        "global_0x14d690_written_at_0x12289e":
            refs.get("0012289e")=="WRITE",
        "global_0x14d690_read_at_0x122838":
            refs.get("00122838")=="READ",
        "download_gate_present":
            "mpf_fastboot_permission()" in body("0x00122980"),
        "flash_gate_present":
            "mpf_fastboot_permission()" in body("0x00122c5c"),
        "erase_gate_present":
            "mpf_fastboot_permission()" in body("0x00122bf8"),
        "boot_completion_calls_bootm":
            "mpf_bootm_command(" in body("0x001226b4"),
        "boot_completion_resets_if_bootm_returns":
            "mpf_reset_cpu_candidate(" in body("0x001226b4"),
        "boot_completion_has_local_permission_check":
            "mpf_fastboot_permission()" in body("0x001226b4"),
        "normal_script_bootz":
            "bootz " in normal and "zImage" in normal,
        "stock_scripts_equal_3_6_and_3_10":
            bool(boot["normalized_commands_equal"]),
        "mmc1_in_saved_targets":"mmc1" in env["boot_targets"],
        "mmc1_disabled_in_embedded_fdt":
            'dwmmc@ff0c0000 {' in fdt and
            'status = "disabled";' in fdt.split("dwmmc@ff0c0000 {",1)[1].split("};",1)[0],
        "az01_bootloader_sig_present_3_10":
            d["boot"]["elfs"]["/usr/bin/az01-bootloader-sig"]["firmware310"]["present"],
        "loadpin_oneshot_present_3_10":
            d["boot"]["services"]["/usr/lib/systemd/system/az0x-loadpin-trustpoints.service"]["firmware310"]["present"],
    }
    checks["flash_partition_capacity_guard"] = (
        "FUN_00110084" in body("0x001100c8")
        and "local_8c < uVar4" in body("0x001100c8")
        and "iVar1 + 0x68" in body("0x001100c8"))
    checks["flash_sparse_four_chunk_kinds"] = (
        "FUN_0010fde8" in body("0x001100c8")
        and all(("case 0x" + code) in body("0x0010fde8")
                for code in ("cac1","cac2","cac3","cac4")))
    checks["sparse_raw_fill_partition_bound"] = (
        "(uint)(param_1[1] + param_1[2]) < iVar11 + uVar2"
        in body("0x0010fde8"))
    checks["erase_partition_block_range_guard"] = (
        "FUN_00110084" in body("0x00110270")
        and "local_8c < uVar5" in body("0x00110270")
        and "iVar2 + 0x6c" in body("0x00110270"))
    probe=u.get("direct_boot_probe") or {}
    # New Ghidra byte-level facts are distinct from existing xref evidence.
    checks["literal_pool_both_pointers_equal"] = (
        probe.get("permission_read_write_pointer_equal") if probe else None
    )
    checks["literal_pool_both_point_to_global"] = (
        probe.get("permission_read_write_pointer_is_prior_global") if probe else None
    )
    checks["published_oem_token_binary_sha256_matches"] = (
        probe.get("oem_token_slot",{}).get(
            "matches_published_37_byte_unlock_token") if probe else None
    )
    dispatch_entries = probe.get("fastboot_command_table", [])
    checks["fastboot_dispatch_table_eight_entries"] = (
        len(dispatch_entries) == 8)
    checks["fastboot_boot_command_callback_resolved"] = any(
        (entry.get("prefix_text") or "").startswith("boot") and
        entry.get("callback_containing_function") == "0x00122964"
        for entry in dispatch_entries
    )
    checks["fastboot_download_command_callback_resolved"] = any(
        (entry.get("prefix_text") or "").startswith("download") and
        entry.get("callback_containing_function") == "0x00122980"
        for entry in dispatch_entries
    )
    rows=[]
    def q(topic,question,result,evidence,next_step):
        rows.append({"id":"DB%03d"%(len(rows)+1),"topic":topic,
                     "question":question,"result":result,"evidence":evidence,
                     "next_step":next_step})
    yes=lambda x:"verified-static" if x else "needs-evidence"
    q("OEM unlock","Is the 37-byte OEM unlock comparison in exact P3?",yes(checks["token_prefix_comparison_37_bytes"]), "0x122854 string comparison of length 0x25.","Resolve literal-pool token hash and pointer targets.")
    q("OEM unlock","Does OEM command set a permission flag?",yes(checks["oem_grants_permission_pointer"]), "0x122854 writes through DAT_00122928.","Trace initialization/reset and persistence separately.")
    q("OEM unlock","Do old P3 Ghidra cross references join grant and permission read?",yes(checks["global_0x14d690_written_at_0x12289e"] and checks["global_0x14d690_read_at_0x122838"]),"Global 0x14D690 has a WRITE ref at 0x12289e and READ at 0x122838.","Check both literal-pool pointers resolve to 0x14D690.")
    q("OEM unlock","Is download gated before receiving bytes?",yes(checks["download_gate_present"]),"0x122980 calls mpf_fastboot_permission.","Trace dispatch and denial status.")
    q("OEM unlock","Are flash and erase each gated?",yes(checks["flash_gate_present"] and checks["erase_gate_present"]),"0x122c5c and 0x122bf8 call permission routine.","Trace partition lookup and write size limits.")
    q("OEM unlock","Is Fastboot boot gated at command dispatch?","unknown","Boot completion at 0x1226b4 does not itself show permission check.","Resolve dispatcher and upstream state machine.")
    q("OEM unlock","Does unlock persist or alter storage?","unknown","Direct grant is a memory write; no proven lifecycle/side effects.","Trace all global writers and data-store calls.")
    q("OEM unlock","Does exact command dispatch map boot and download handlers?",
      yes(checks["fastboot_dispatch_table_eight_entries"] and
          checks["fastboot_boot_command_callback_resolved"] and
          checks["fastboot_download_command_callback_resolved"]),
      "The eight-entry prefix table must map boot to 0x122964 and download to 0x122980. Its exact entries are private Ghidra evidence.",
      "Inspect callback side effects, global permission gates and any boot preconditions.")
    q("RAM boot","Does a locked session have an authorized upload route?",
      "blocked-by-download-gate",
      "The download handler checks the permission flag even though boot completion has no local check. This does not exclude an alternative upload path.",
      "Trace every dispatch-table callback for a different way to populate the Fastboot image buffer.")
    q("RAM boot","Is there a Fastboot RAM bootm completion path?",yes(checks["boot_completion_calls_bootm"]),"0x1226b4 invokes 0x101744 bootm.","Prove image header acceptance on this exact path.")
    q("RAM boot","Does Fastboot boot return to a CPU reset?",yes(checks["boot_completion_resets_if_bootm_returns"]),"0x1226b4 invokes reset after bootm returns.","Interpret failed-image flow; no actual boot attempt.")
    q("RAM boot","Does ordinary boot use raw zImage via bootz?",yes(checks["normal_script_bootz"]),"3.10 boot script loads zImage/FDT and invokes bootz.","Inspect loader address, FDT, bootargs and rootfs UUID.")
    q("RAM boot","Are bootm and bootz interchangeable?","no","General bootm recognizes wrapped legacy/FIT formats; bootz raw Linux zImage.","Analyze valid synthetic legacy image and FIT failure branches.")
    q("RAM boot","Does maximum Fastboot download equal safe physical RAM?","no","0x70000000 check is protocol bound, not proof of free RAM.","Map relocation, download base and FDT/kernel/ramdisk overlaps.")
    q("RAM boot","Does a download near 23 MiB approach configured FDT placement?","needs-memory-proof",
        "FDT address minus loadaddr is "+str(nominal_memory_gaps["fdt_addr_r"]["from_fastboot_load_base_bytes"])+
        " bytes; this is a nominal gap, not a measured live overlap.",
        "Verify U-Boot download area, relocation, FDT residency and guard checks before any real boot.")

    q("External media","Does U-Boot scan MMC1?",yes(checks["mmc1_in_saved_targets"]),"Saved boot_targets includes mmc0 mmc1.","Check whether runtime enables the removable controller.")
    q("External media","Is MMC1 disabled in U-Boot DTB?",yes(checks["mmc1_disabled_in_embedded_fdt"]),"MMC1 ff0c0000 disabled in embedded FDT.","Trace driver startup and runtime DTS fixups.")
    q("External media","Can owner boot SD without persistent environment change?","unknown","No proof of SD boot or interactive nonpersistent selection on MPC One.","Find safe serial/boot mechanism; no P2 writes.")
    q("Storage safety","Does raw Fastboot flash check a partition capacity?",
      yes(checks["flash_partition_capacity_guard"]),
      "Function 0x1100c8 resolves a partition and compares aligned requested extent against its available count before the storage write callback.",
      "Trace integer overflow, partition lookup and actual eMMC physical-range mapping.")
    q("Storage safety","Does this U-Boot recognize Android sparse images?",
      yes(checks["flash_sparse_four_chunk_kinds"]),
      "0x10fde8 has RAW/FILL/DONT_CARE/CRC32 chunk-type branches, and a separate raw/fill output-range check.",
      "Analyze chunk-count arithmetic and missing/invalid sparse metadata cases.")
    q("Storage safety","Does sparse image raw/fill output have a capacity condition?",
      yes(checks["sparse_raw_fill_partition_bound"]),
      "0x10fde8 compares accumulated write end against partition-end for selected chunk types; other paths still require inspection.",
      "Verify all chunk types, integer overflow and final extent handling with ARM instructions.")
    q("Storage safety","Does partition erase have an aligned extent guard?",
      yes(checks["erase_partition_block_range_guard"]),
      "0x110270 computes aligned start/end values, checks partition extent and invokes erase callback.",
      "Trace every callback and distinguish physical block addresses from filesystem names.")
    q("Independent rootfs","Are 3.6 and 3.10 boot scripts unchanged?",yes(checks["stock_scripts_equal_3_6_and_3_10"]),"Normalized scripts equal and call bootz.","Test matching rootfs boot layout in disposable ARM QEMU.")
    q("Independent rootfs","Does openAZ0x have MPC ACVA configuration?","upstream-only","meta-az0x supports acva config, but author marks MPC hardware untested.","Compare kernel 6.18.44 owner rootfs vs openAZ0x 6.18.26 default.")
    q("Independent rootfs","Can MPFree preserve P1/P2/P3?","architecture-supported","Vendor U-Boot loads kernel and DTB from rootfs partition P6.","Qualify P6-only image offline and test kexec before installing.")
    q("Boot counter","Is 0xFF7300A0 warm reboot policy present in MPC P3?","unknown","openAZ0x observed it on JC11S; not proven on original ACVA P3.","Search full Ghidra register read/write and OS acknowledgment.")
    q("Boot counter","Can synthetic B001 counter stand in for MPC tests?","related-board-only","B001 format reported on JC11S, not validated MPC threshold.","Keep board hypothesis distinct from device proof.")
    q("3.10 policy","Does 3.10 add a bootloader signature utility?",yes(checks["az01_bootloader_sig_present_3_10"]),"az01-bootloader-sig added in 3.10.","Trace whether updater/boot scripts call it.")
    q("3.10 policy","Does 3.10 add an early LoadPin service?",yes(checks["loadpin_oneshot_present_3_10"]),"az0x-loadpin-trustpoints systemd oneshot exists.","Trace kernel enforcement and service dependency.")
    q("3.10 policy","Does the OEM Fastboot grant bypass RSA cryptography?","no","Fastboot permission is separate from updater authentication and kernel verification.","Trace actual normal bootz path without confusing security layers.")
    q("MaskROM/recovery","Does official MPC One Fastboot recovery work without rootfs?","public-reported","RedHate documents entering stock U-Boot Fastboot via hardware buttons.","Do read-only enumeration on owner's unit after explicit approval.")
    q("MaskROM/recovery","Does Fastboot guarantee recovery after SPL/P3 corruption?","no","Fastboot requires working vendor SPL and U-Boot.","Prove immutable RK3288 ROM entry and independent restore before altering P1/P3.")
    return {"schema":SCHEMA,"checks":checks,"questions":rows,
            "probe":probe,"fastboot_dispatch_entries":dispatch_entries,
            "private_source_sha256":UBOOT_SHA,
            "fastboot_load_base_address": "0x%08x" % load_base,
            "nominal_memory_address_gaps":nominal_memory_gaps,
            "fastboot_download_memory_safe_proven":False,
            "physical_fastboot_commands_sent":False,
            "vendor_code_executed":False}

def render_md(d):
    lines=["# MPFree direct-boot research evidence","",
           "Read-only, from owner Ghidra/boot reports; no code executed.",
           "Exact P3 SHA-256: "+d["private_source_sha256"],"",
           "## Automated static checks"]
    for name,ok in d["checks"].items():
        lines.append("- "+name+": "+str(ok))
    lines.extend(["","## Nominal download-to-kernel/FDT/ramdisk distances","",
                  "Load base: "+d["fastboot_load_base_address"],
                  "These are NOT verified safe download sizes.","",
                  json.dumps(d["nominal_memory_address_gaps"],indent=2),""])
    lines.extend(["","## OEM literal-pool / PMU probe","",
                  json.dumps(d["probe"],indent=2), ""])
    for topic in dict.fromkeys(x["topic"] for x in d["questions"]):
        lines.extend(["","## "+topic,""])
        for row in d["questions"]:
            if row["topic"]==topic:
                lines.extend(["### "+row["id"]+" "+row["question"],
                              "Status: "+row["result"],"",
                              "Evidence: "+row["evidence"],"",
                              "Next: "+row["next_step"],""])
    return "\n".join(lines)+"\n"

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--integrated-zip",required=True,type=Path)
    p.add_argument("--json",type=Path)
    p.add_argument("--markdown",type=Path)
    args=p.parse_args()
    if args.integrated_zip.is_symlink() or not args.integrated_zip.is_file() or not stat.S_ISREG(args.integrated_zip.lstat().st_mode):
        p.error("Expected a private regular ZIP file")
    with zipfile.ZipFile(args.integrated_zip) as z:
        if z.testzip():raise ValueError("ZIP CRC mismatch")
        index=json.loads(z.read("ZIP_INDEX.json"))
        if index.get("schema")!="mpfree-bootchain-integrated-zip-index-v1":
            raise ValueError("Unexpected private ZIP schema")
        members={x["name"]:x for x in index["files"]}
        if set(z.namelist())!=set(members)|{"ZIP_INDEX.json"}:
            raise ValueError("Unindexed or missing ZIP member")
        for file in SOURCE.values():
            entry=members[file]
            blob=z.read(file)
            if len(blob)!=entry["bytes"] or hashlib.sha256(blob).hexdigest()!=entry["sha256"]:
                raise ValueError("Source hash mismatch: "+file)
        result=build(z.read)
    for path,body in ((args.json,json.dumps(result,indent=2)+"\n"),
                      (args.markdown,render_md(result))):
        if path:
            if path.exists() or path.is_symlink():raise FileExistsError(str(path))
            path.parent.mkdir(parents=True,exist_ok=True)
            path.write_text(body)
    print("DIRECT_BOOT_EVIDENCE:",len(result["questions"]),"questions",
          len(set(x["topic"] for x in result["questions"])),"tracks")
    print("OEM checks:",json.dumps(result["checks"],sort_keys=True))
if __name__=="__main__":
    main()
