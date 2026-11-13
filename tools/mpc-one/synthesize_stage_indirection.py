#!/usr/bin/env python3
"""Cross-stage MPC SPL indirection, Fastboot power-source and updater linkage.

Read-only synthesis over already SHA-indexed Ghidra outputs.
Vendor images, code execution, external USB, and emulated RK3288
ROM hardware are not part of this program.
"""
from __future__ import annotations
import json

SCHEMA="mpfree-cross-stage-indirection-v1"
INPUTS={
    "spl":"static/v11/v11-ghidra-spl.json",
    "uboot":"static/v11/v11-ghidra-uboot.json",
    "updater":"static/v11/v11-ghidra-az01-update.json",
}
# Keys name logical analysis stages; V11 reports keep their real firmware role.
# In particular the "updater" stage is the "az01-update" executable.
EXPECTED_SOURCE_ROLES={
    "spl":"spl",
    "uboot":"uboot",
    "updater":"az01-update",
}

def build(read):
    docs={role:json.loads(read(path)) for role,path in INPUTS.items()}
    for role,doc in docs.items():
        if (doc.get("role")!=EXPECTED_SOURCE_ROLES[role]
            or len(str(doc.get("source_sha256_prechecked_by_launcher","")))!=64):
            raise ValueError("Missing source SHA/role: "+role)
    spl=docs["spl"]
    up=docs["updater"]
    u=docs["uboot"]
    p=spl.get("direct_boot_probe",{})
    upl=up.get("direct_boot_probe",{})
    if p.get("role")!="spl" or upl.get("role")!="az01-update":
        raise ValueError("Original pointer roles are not established")
    spl_fun=spl.get("analyzed_functions",{})
    ufun=u.get("analyzed_functions",{})
    entry=spl_fun.get("0xff705208",{}).get("pseudocode","")
    selector=spl_fun.get("0xff705350",{}).get("pseudocode","")
    second=spl_fun.get("0xff705310",{}).get("pseudocode","")
    getvar=ufun.get("0x00100ef0",{}).get("pseudocode","")
    fastboot_getvar=ufun.get("0x00122d38",{}).get("pseudocode","")
    stage_ext=p.get("extended_stage_evidence",{})
    stage_ptr=p["original_image_pointers"]
    first_target=stage_ptr["candidate_stage_entry_handoff"]["original_word"]
    first_matches=(first_target=="0xff705351"
                   and "(*DAT_ff705274)(" in entry
                   and bool(stage_ext.get("first_indirect_call_matches_boot_selector")))
    context=stage_ptr["stage_handoff_param"]["original_word"]
    second_slot=("0x%08x" % (int(context,16)+12)) if context else None
    followup=stage_ext.get("indirect_handoff_original_word")
    media_tables=stage_ext.get("opaque_media_table_words",{})
    mapping=media_tables.get("device_mapping",{})
    order=media_tables.get("device_order",{})
    contiguous=(
        bool(mapping.get("table_start") and order.get("table_start"))
        and int(mapping["table_start"],16)+4*len(
            mapping.get("entries",[])) == int(order["table_start"],16)
        and len(mapping.get("entries",[]))==8
        and len(order.get("entries",[]))==5
    )
    decoder_checks={
        "initial_spl_indirect_function_pointer_matches_selector":first_matches,
        "selector_calls_device_loader":("FUN_ff7054a4()" in selector),
        "selector_passes_context_to_indirect_handler":(
            "FUN_ff705310(DAT_ff70540c)" in selector),
        "second_handoff_indirect_dereference_observed":(
            "(**(code **)(param_1 + 0xc))()" in second),
        "second_handoff_slot_is_context_plus_0x0c":(
            stage_ext.get("indirect_handoff_function_pointer_slot")==second_slot
            and second_slot is not None),
        "next_indirect_word_statically_captured":followup is not None,
        "five_device_order_entries_captured":len(
            stage_ext.get("opaque_media_table_words",{}).get(
                "device_order",{}).get("entries",[]))==5,
        "eight_media_mapping_entries_captured":len(
            stage_ext.get("opaque_media_table_words",{}).get(
                "device_mapping",{}).get("entries",[]))==8,
        "mapping_table_ends_at_device_order_start":contiguous,
    }
    literals=u.get("direct_boot_probe",{}).get(
        "reboot_mode_original_literal_slots",{})
    key=literals.get("getvar_adc_key_string",{}).get("printable_string")
    power_source={
        "getvar_query_key":key,
        "power_source_key_verified":key=="power-source",
        "fastboot_getvar_dispatches_to_adc_probe":(
            "FUN_00100ef0(local_70,auStack_4c" in fastboot_getvar),
        "saradc_channel_0_used":all(t in getvar for t in (
            "FUN_001147f0(10,0,&local_24)",
            "FUN_00115ef4(local_24,0)",
            "FUN_00115f30(local_24,0,&local_20)",
            "FUN_00115f90(local_24,aiStack_1c)")),
        "voltage_or_adc_scale_calculation_present":(
            "local_20 * aiStack_1c[0]" in getvar
            and "DAT_00100fc4 >> 0x26" in getvar),
        "power_source_label_original_pointers":{
            label:literals.get(label)
            for label in (
                "getvar_power_source_label_1_string",
                "getvar_power_source_label_2_string",
                "getvar_power_source_label_3_string",
                "getvar_power_source_label_4_string")},
        "power_source_threshold_original_words":{
            label:literals.get(label)
            for label in (
                "getvar_power_source_threshold_1",
                "getvar_power_source_threshold_2",
                "getvar_power_source_threshold_3")},
        "adc_code_1_recovery_0_power_source":True,
        "runtime_voltages_measured":False,
    }
    updater_data=upl.get("original_image_pointers",{})
    names={
        "usb_update_mode_trampoline":"az01_usb_update_mode",
        "apply_image_trampoline":"az01_apply_image",
        "download_image_trampoline":"az01_download_image",
    }
    import_linkage={}
    for key,function_name in names.items():
        item=updater_data.get(key,{})
        target=item.get("possible_code_target")
        f=up.get("analyzed_functions",{}).get(target,{})
        pseudo=f.get("pseudocode","")
        unresolved=("halt_baddata" in pseudo or
                    "Bad instruction" in pseudo or not pseudo)
        import_linkage[key]={
            "original_pointer":item.get("original_word"),
            "stub_address":target,
            "named_stub":f.get("name",function_name),
            "stub_decompilation_bad_or_missing":unresolved,
            "dynamic_library_implementation_resolved":False,
            "vendor_updater_executed":False,
        }
    return {
        "schema":SCHEMA,
        "source_images_sha256":{
            k:doc["source_sha256_prechecked_by_launcher"]
            for k,doc in docs.items()},
        "spl":{
            "first_indirect_target":first_target,
            "first_indirect_boot_selector_proven":first_matches,
            "next_handoff_context":context,
            "next_handoff_function_slot":second_slot,
            "next_handoff_word_original_image":followup,
            "next_handoff_runtime_target_proven":False,
            "opaque_media_tables":media_tables,
            "mapping_and_order_tables_adjacent_no_overlap":contiguous,
            "checks":decoder_checks,
        },
        "fastboot_getvar_power_source":power_source,
        "updater_import_linkage":import_linkage,
        "updater_dynamic_library_execution_proven":False,
        "bootrom_initial_sram_execution_proven":False,
        "hardware_sd_boot_proven":False,
        "physical_hardware_access":False,
        "original_vendor_code_executed":False,
        "next_experiments":[
            "Resolve SPL second handoff at context+0x0c without mistaking a mutable context for an immutable target.",
            "Decode five boot-order words and eight mapping words against actual driver selection cases and media aliases.",
            "Inspect power-source ADC channel-0 original thresholds, scale and four response strings.",
            "Resolve imported az01-update functions to libaz01-prog implementations using ELF relocation/symbol metadata.",
            "Continue independent ROM/USB entry and all-device restoration research.",
        ],
    }

def markdown(doc):
    lines=["# MPFree cross-stage pointer/ADC/linkage audit","",
           "Offline source-only; physical hardware and vendor execution never used.","",
           "## SPL first and second indirection"]
    spl=doc["spl"]
    for k in ("first_indirect_target","first_indirect_boot_selector_proven",
              "next_handoff_context","next_handoff_function_slot",
              "next_handoff_word_original_image",
              "next_handoff_runtime_target_proven"):
        lines.append("- "+k+": "+str(spl[k]))
    lines.extend(["","## Fastboot getvar power-source ADC","",
                  json.dumps(doc["fastboot_getvar_power_source"],indent=2),
                  "","## Update import stubs (NOT resolved implementation)","",
                  json.dumps(doc["updater_import_linkage"],indent=2),
                  "","## Open questions"])
    lines.extend("- "+x for x in doc["next_experiments"])
    return "\n".join(lines)+"\n"
