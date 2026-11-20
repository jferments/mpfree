#!/usr/bin/env python3
"""Owner-private offline source crosscheck: image formats, verify state and SPL.

Only reads SHA-indexed Ghidra evidence. Does not execute vendor firmware or
connect to USB, ADC, eMMC or immutable BootROM.
"""
from __future__ import annotations
import json

SCHEMA="mpfree-update-format-spl-followup-v1"
FILES={
    "lib":"static/v11/v11-ghidra-libaz01-prog.json",
    "spl":"static/v11/v11-ghidra-spl.json",
    "usb":"usb-backend-callback-coverage.json",
}

def verification_state_step(state:int, option:int, callback_result:int)->dict:
    """Model ONLY verified option values 0 and 2, and states 0,1,2."""
    if state not in (0,1,2) or option not in (0,2):
        raise ValueError("Outside verified state/option subset")
    if state==1 or (state!=0 and option==2):
        return {"callback_called":False,"result":0,"new_state":state}
    if not isinstance(callback_result,int):
        raise TypeError("Callback result must be an integer")
    if callback_result==0:
        return {"callback_called":True,"result":0,"new_state":1}
    if callback_result==-13:
        return {"callback_called":True,"result":-13,"new_state":2}
    return {"callback_called":True,"result":callback_result,"new_state":state}

def build(read):
    d={k:json.loads(read(path)) for k,path in FILES.items()}
    lib,spl,usb=(d[k] for k in ("lib","spl","usb"))
    def role(doc,expected):
        sha=doc.get("source_sha256_prechecked_by_launcher")
        if doc.get("role")!=expected or not isinstance(sha,str) or len(sha)!=64:
            raise ValueError("Unverified original source role "+expected)
        return sha
    lsha=role(lib,"libaz01-prog")
    ssha=role(spl,"spl")
    if (usb.get("original_library_sha256")!=lsha
        or not usb.get("all_source_only_backends_matched")):
        raise ValueError("USB backend source/role not independently matched")
    functions=lib["analyzed_functions"]
    def fn(addr,name):
        info=functions.get(addr,{})
        if info.get("name")!=name or not info.get("pseudocode"):
            raise ValueError("Original implementation unavailable: "+name)
        return info["pseudocode"]
    opener=fn("0x0001738c","az01_open_image")
    verify=fn("0x00017554","az01_verify_image")
    writer=fn("0x0001798c","az01_write_partition_image")
    stream=fn("0x00014d00","az01_stream_partition")
    checks={
        "opens_AZ0x":all(x in opener for x in (
            "*pcVar3 == 'A'","pcVar3[1] == 'Z'",
            "pcVar3[2] == '0'","pcVar3[3] == 'x'")),
        "opens_AZ01":all(x in opener for x in (
            "pcVar3[1] != 'Z'","pcVar3[2] != '0'","pcVar3[3] != '1'")),
        "recognizes_FDT_magic_d00dfeed":all(x in opener for x in (
            "*pcVar3 != -0x30","pcVar3[1] != '\\r'",
            "pcVar3[2] != -2","pcVar3[3] != -0x13")),
        "format_dispatch_indirect":("*puVar4" in opener and
                                    "(*pcVar2)(__ptr)" in opener),
        "verification_prechecked_state_and_option2":all(
            x in verify for x in (
                "param_1[7] == 1","param_1[7] != 0","param_2 >> 1")),
        "verification_sets_state_1_on_success":(
            "if (iVar1 == 0)" in verify and "param_1[7] = 1;" in verify),
        "verification_marks_minus13_as_state2":(
            "if (iVar1 == -0xd)" in verify and "param_1[7] = 2;" in verify),
        "writes_uncompressed_bootloader":(
            "az01_write_bootloader(param_1,param_2[5],param_2[4],param_3)"
            in writer),
        "writes_uncompressed_partition":(
            "az01_write_partition(param_1,param_2[1],param_2[5],param_2[4])"
            in writer),
        "XZ_codec_branch":all(x in writer for x in (
            "*__s1 == 'x'","__s1[1] == 'z'","__s1[2] == '\\0'")),
        "XZ_bootloader_10MiB_check":all(x in writer for x in (
            "(0xa00000 < local_3c)","malloc(local_3c)",
            "lzma_stream_buffer_decode")),
        "XZ_partition_stream":all(x in writer for x in (
            "lzma_stream_decoder","pthread_create",
            "az01_stream_partition","pthread_join")),
        # &local_e0 is immediately before local_dc[0]. The loop
        # increments BEFORE allocation and exits at local_dc+3:
        # four buffers, indices [0], [1], [2], [3], not three.
        "XZ_partition_4_x_1MiB_chunks":all(x in writer for x in (
            "malloc(0x100000)","ppiVar4 = &local_e0;",
            "ppiVar4 = ppiVar4 + 1;",
            "while (ppiVar4 != local_dc + 3)")),
        "stream_null_callback_returns_minus11":(
            "UNRECOVERED_JUMPTABLE == (code *)0x0" in stream and
            "return 0xfffffff5;" in stream),
    }
    backends={b["descriptor"]:b for b in usb["backends"]}
    required=("ff/42/03","ff/61/7a","ff/06/05")
    def has_stream(name):
        return "stream_partition" in backends.get(name,{}).get(
            "present_operations",[])
    checks["RockUSB_like_backend_lacks_stream_slot"]=(
        all(name in backends for name in required)
        and has_stream("ff/42/03") and has_stream("ff/61/7a")
        and not has_stream("ff/06/05"))
    spl_funcs=spl["analyzed_functions"]
    resolver=spl_funcs.get("0xff704120",{}).get("pseudocode","")
    selector=spl_funcs.get("0xff705350",{}).get("pseudocode","")
    checks["SPL_source_resolver_only_1_or_8"]=all(
        x in resolver for x in ("uVar2 = 1;","uVar2 = 8;","iVar1 == 0x35"))
    checks["SPL_first_order_entry_overwritten"]=(
        "FUN_ff705342(DAT_ff7053fc)" in selector)
    return {
        "schema":SCHEMA,
        "original_elf_sha256":lsha,"original_spl_sha256":ssha,
        "checks":checks,
        "all_checkable_static_claims_passed":all(checks.values()),
        "image_format_prefixes":[
            "AZ0x","AZ01","D0 0D FE ED (FDT header; FIT candidate only)"],
        "FDT_image_must_be_valid_FIT_proven":False,
        "original_format_dispatch_PIC_slots":(
            (lib.get("direct_boot_probe") or {}).get(
                "format_handler_candidate_original_words")),
        "computed_original_format_descriptor_slots":(
            (lib.get("direct_boot_probe") or {}).get(
                "computed_PIC_descriptor_table_slots")),
        "format_handler_live_target_proven":False,
        "verification_state_examples":{
            "fresh_0_option0_error_minus13":verification_state_step(0,0,-13),
            "reused_2_option2":verification_state_step(2,2,-13),
            "reused_2_option0_error":verification_state_step(2,0,-13),
        },
        "minus13_error_semantics_proven":False,
        "reuse_same_image_object_across_update_paths_proven":False,
        "xz_bootloader_check_max_bytes_on_this_path":0xa00000,
        "xz_partition_working_buffer_bytes_each":0x100000,
        "xz_partition_buffer_count_observed":4,
        "xz_stream_via_RockUSB_generic_backend_supported":False,
        "SPL_source_resolver_candidate_codes":[1,8],
        "SPL_code_8_physical_media_identified":False,
        "original_update_or_device_code_executed":False,
        "physical_maskrom_or_emmc_restore_proven":False,
    }

def markdown(d):
    lines=["# MPFree offline image-format, verification and SPL study","",
           "SHA-indexed original ARM evidence only; no vendor execution.","",
           "## Static source checks"]
    lines.extend("- "+k+": "+str(v) for k,v in d["checks"].items())
    lines.extend(["","## Interpretation limits",
        "- FDT magic does not prove FIT acceptance or a signature policy.",
        "- A cached -13 state and option 2 do not prove a practical bypass.",
        "- XZ size bound applies only to the identified bootloader code path.",
        "- Missing USB stream callback does not rule out alternate protocols.",
        "- SPL code 8 has not been mapped to a physical connector.",""])
    return "\n".join(lines)
