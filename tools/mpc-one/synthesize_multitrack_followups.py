#!/usr/bin/env python3
"""SHA-pinned source-only SPL, ADC and 3.10 updater flow synthesis.

Reads prior owner's private, ZIP-indexed Ghidra and readelf output.
Never runs firmware, invokes physical USB or touches device storage.
"""
from __future__ import annotations
import json

SCHEMA="mpfree-source-multitrack-followups-v1"
PATHS={
    "spl":"static/v11/v11-ghidra-spl.json",
    "p1":"static/wide/p1-sram-ghidra.json",
    "uboot":"static/v11/v11-ghidra-uboot.json",
    "lib":"static/v11/v11-ghidra-libaz01-prog.json",
    "elf":"static/v11/v11-elf-linkage-libaz01-prog.json",
    "links":"elf-relocation-crosscheck.json",
}
LABELS=("invalid","battery","ac","invalid-ac")
BOUNDS=(624999,1089999,1249999)
SCALE=0x10624dd3
EXPECTED_ROM_RECOVERY_PROVEN=False


def classify_uint32(value:int)->str:
    """Reproduce the original mixed unsigned/signed comparisons, not volts."""
    if not 0<=value<2**32:
        raise ValueError("Power-source result must be uint32")
    if value<=BOUNDS[0]:
        return LABELS[0]
    signed=value if value<2**31 else value-2**32
    if signed<=BOUNDS[1]:
        return LABELS[1]
    if signed<=BOUNDS[2]:
        return LABELS[2]
    return LABELS[3]


def adc_fixed_point(adc:int,reference:int)->int:
    """Emulate decompiled unsigned 32-bit product then 64-bit multiply+shift."""
    if not all(isinstance(x,int) and 0<=x<2**32 for x in (adc,reference)):
        raise ValueError("Expected two uint32 ADC/reference inputs")
    product=(adc*reference)&0xffffffff
    return (product*SCALE)>>38


def build(read):
    d={key:json.loads(read(path)) for key,path in PATHS.items()}
    spl,p1,ub,lib,elf,links=(d[k] for k in
                             ("spl","p1","uboot","lib","elf","links"))
    def check_role(doc,role):
        source=doc.get("source_sha256_prechecked_by_launcher")
        if doc.get("role")!=role or not isinstance(source,str) or len(source)!=64:
            raise ValueError("Expected verified original "+role+" Ghidra input")
        return source
    ssha=check_role(spl,"spl")
    usha=check_role(ub,"uboot")
    lsha=check_role(lib,"libaz01-prog")
    if p1.get("sha256")!=ssha or elf.get("source_sha256_verified")!=lsha:
        raise ValueError("P1 strings or ELF exports differ from Ghidra source")
    if links.get("owner_sources_sha256",{}).get("libaz01-prog")!=lsha:
        raise ValueError("Cross-stage ELF source provenance mismatch")
    f=spl["analyzed_functions"]
    stage=spl["direct_boot_probe"]["extended_stage_evidence"]
    t=stage["opaque_media_table_words"]
    order=t["device_order"]["entries"]
    mapping=t["device_mapping"]["entries"]
    if len(order)!=5 or len(mapping)!=8:
        raise ValueError("SPL table sizes changed")
    strings={x["at"].lower():x["text"] for x in p1["strings"]
             if "at" in x and "text" in x}
    mapping_labels=[]
    for i in range(0,8,2):
        key=int(mapping[i]["word"],16)
        ptr=mapping[i+1]["word"].lower()
        mapping_labels.append({
            "numeric_id":key,
            "pointer_address":ptr,
            "original_string":strings.get(ptr),
            "physical_mpc_controller_proven":False,
        })
    selector=f["0xff705350"]["pseudocode"]
    writes=f["0xff705342"]["pseudocode"]
    spl_checks={
        "boot_order_five_original_words_are_deadbeef":all(
            x["word"].lower()=="0xdeadbeef" for x in order),
        "three_id_to_mmc_label_pairs":(
            [(x["numeric_id"],x["original_string"]) for x in mapping_labels[:3]]
            == [(1,"MMC1"),(2,"MMC2"),(3,"MMC2_2")]),
        "fourth_pair_has_sentinel_identifier":(
            mapping_labels[-1]["numeric_id"]==0xdeadbeef),
        "selector_calls_first_boot_order_writer":(
            "FUN_ff705342(DAT_ff7053fc)" in selector),
        "first_order_writer_calls_source_resolver":(
            "FUN_ff704120()" in writes and "*param_1 = uVar1;" in writes),
        "context_second_callback_initially_zero":(
            stage["indirect_handoff_original_word"]=="0x00000000"),
    }
    uslots=ub["direct_boot_probe"]["reboot_mode_original_literal_slots"]
    def word(key):
        x=uslots.get(key,{}).get("value")
        return int(x,16) if isinstance(x,str) else None
    constants=[word("getvar_power_source_threshold_"+str(i))
               for i in range(1,4)]
    labels=[uslots.get("getvar_power_source_label_"+str(i)+"_string",{}).get(
                "printable_string") for i in range(1,5)]
    getvar=ub["analyzed_functions"]["0x00100ef0"]["pseudocode"]
    power_checks={
        "all_four_original_response_labels":labels==list(LABELS),
        "three_exact_boundary_constants":constants==list(BOUNDS),
        "scale_constant_divides_by_approximately_1000":(
            word("getvar_power_source_scale")==SCALE and
            abs(SCALE*1000/(2**38)-1)<1e-8),
        "original_code_uses_mixed_unsigned_signed_compares":all(
            x in getvar for x in
            ("if (DAT_00100fc8 < uVar3)",
             "if (DAT_00100fd0 < (int)uVar3)",
             "if (DAT_00100fd8 < (int)uVar3)")),
        "original_code_multiplies_and_shifts":(
            "local_20 * aiStack_1c[0]" in getvar
            and ">> 0x26" in getvar),
    }
    libf=lib["analyzed_functions"]
    def pseudo(addr,expected_name):
        row=libf.get(addr,{})
        if row.get("name")!=expected_name:
            raise ValueError("Missing original implementation: "+expected_name)
        return row.get("pseudocode","")
    download=pseudo("0x0001b9bc","az01_download_image")
    verify=pseudo("0x00017554","az01_verify_image")
    signature=pseudo("0x0001f094","az01_verify_signature")
    write=pseudo("0x0001e194","az01_write_image")
    partition=pseudo("0x0001798c","az01_write_partition_image")
    callback_specs=[
        ("0x00014ac8","az01_commit_bootloader","+ 0x2c"),
        ("0x00014b98","az01_write_bootloader","+ 0x18"),
        ("0x00014c50","az01_write_partition","+ 0x1c"),
    ]
    callbacks={name:("+ 0x18" in pseudo(addr,name) and offset in
                     pseudo(addr,name) and "UNRECOVERED_JUMPTABLE" in
                     pseudo(addr,name))
               for addr,name,offset in callback_specs}
    updater_checks={
        "three_updater_import_export_pairs":all(
            x["imported_by_updater"] and x["defined_by_libaz01_prog"]
            for x in links["target_update_functions"]),
        "onboard_key_lookup_precedes_download_verification":(
            download.find("az01_image_set_onboard_key_lookup(")>=0 and
            download.find("az01_image_set_onboard_key_lookup(")<
            download.find("az01_verify_image(local_1064,0)")<
            download.find("az01_apply_image(")),
        "normal_image_write_verifies_before_storage_prepare":(
            0<=write.find("az01_verify_image(param_1,2)")<
               write.find("az01_prepare_to_write(")),
        "compatibility_check_is_flag_dependent":(
            "if ((param_3 & 2) == 0)" in write
            and "az01_image_is_compatible" in write),
        "image_verify_uses_cached_state_flag":all(
            x in verify for x in
            ("param_1[7] == 1","param_2 >> 1",
             "param_1[7] = 1","param_1[7] = 2")),
        "openssl_sha256_256byte_signature_verify":all(
            x in signature for x in
            ("EVP_sha256()","EVP_DigestVerifyInit(",
             "EVP_DigestVerify(ctx,param_4,0x100,param_2,param_3)")),
        "partition_writer_routes_multiple_backend_methods":all(
            x in partition for x in
            ("az01_write_bootloader(","az01_write_partition(",
             "az01_stream_partition")),
        "storage_commit_and_write_callbacks_are_indirect":all(
            callbacks.values()),
    }
    return {
        "schema":SCHEMA,
        "original_sources_sha256":{"spl":ssha,"uboot":usha,"libaz01-prog":lsha},
        "spl":{
            "checks":spl_checks,
            "mapping_name_pairs":mapping_labels,
            "original_boot_order_words":[x["word"] for x in order],
            "first_order_entry_written_before_iteration":spl_checks[
                "selector_calls_first_boot_order_writer"] and spl_checks[
                "first_order_writer_calls_source_resolver"],
            "second_callback_original_word":stage["indirect_handoff_original_word"],
            "second_callback_runtime_target_proven":False,
            "real_boot_media_priority_proven":False,
        },
        "power_source":{
            "checks":power_checks,"original_response_labels":labels,
            "original_threshold_words":constants,
            "fixed_point_multiplier":SCALE,
            "shift_bits":38,
            "scaled_multiplier_divisor":SCALE/(2**38),
            "uint32_boundaries":[{
                "scaled_value":value,
                "result":classify_uint32(value)}
                for value in (0,624999,625000,1089999,1090000,1249999,1250000)],
            "reported_physical_voltage_units_proven":False,
            "runtime_adc_samples_collected":False,
        },
        "updater":{
            "checks":updater_checks,
            "indirect_callback_locations":{
                name:offset for _,name,offset in callback_specs},
            "signature_size_argument_bytes":256,
            "openssl_digest":"SHA-256",
            "verification_cache_and_flags_mean_universal_enforcement_unproven":True,
            "actual_backend_write_invoked":False,
            "actual_signature_acceptance_tested":False,
            "immutable_rom_signature_policy_proven":False,
        },
        "usb_maskrom":{
            "host_three_usb_backends_observed_previously":True,
            "physical_maskrom_entry_proven":False,
            "independent_full_emmc_restore_proven":False,
        },
        "all_static_preconditions_passed":all(
            all(group["checks"].values()) for group in
            ({"checks":spl_checks},{"checks":power_checks},{"checks":updater_checks})),
        "vendor_firmware_executed":False,
        "physical_mpc_accessed":False,
    }


def markdown(report):
    lines=["# MPFree owner-static SPL/ADC/updater followups","",
           "No vendor execution or physical device access.",""]
    for group in ("spl","power_source","updater"):
        row=report[group]
        lines.extend(["## "+group,""])
        for key,value in row["checks"].items():
            lines.append("- "+key+": "+str(value))
        lines.append("")
    lines.extend(["## Classification boundaries","",
                  json.dumps(report["power_source"]["uint32_boundaries"],indent=2),
                  "","## SPL named media identifiers","",
                  json.dumps(report["spl"]["mapping_name_pairs"],indent=2),
                  "","Note: named MMC labels are not proof of physical SD boot.",
                  "A verified local signature helper is not proof of every update path's policy.",
                  "The second SPL callback starts zero and its runtime initializer is unknown.",
                  ""])
    return "\n".join(lines)
