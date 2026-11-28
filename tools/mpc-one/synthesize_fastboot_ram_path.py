#!/usr/bin/env python3
"""Trace Fastboot download->RAM->bootm from SHA-verified U-Boot evidence.

Static source-only: never connects to USB, runs vendor binaries, or accesses
eMMC, /dev/mem or physical MaskROM.
"""
import json

SCHEMA="mpfree-fastboot-ram-path-static-v1"
UBOOT="static/v11/v11-ghidra-uboot.json"
EXPECTED_SHA="b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
RAM_BASE=0x00800800
MAX_ADVERTISED=0x70000000

def build(read):
    d=json.loads(read(UBOOT))
    if (d.get("role")!="uboot"
        or d.get("source_sha256_prechecked_by_launcher")!=EXPECTED_SHA):
        raise ValueError("U-Boot source not identical to authenticated original")
    f=d["analyzed_functions"]
    def body(addr):
        return f.get(addr,{}).get("pseudocode","")
    receiver=body("0x00122a10")
    request=body("0x00122980")
    boot=body("0x00122964")
    completion=body("0x001226b4")
    classify=body("0x0010e390")
    loader=body("0x0010a6a4")
    checks={
        "download_permission_gate":"mpf_fastboot_permission()" in request,
        "zero_or_above_max_length_refused":(
            "*puVar1 == 0" in request and
            "*puVar1 < 0x70000001" in request),
        "receive_copies_to_fixed_ram_base":(
            "&DAT_00800800 + *DAT_00122ac4" in receiver and
            "mpf_memcpy_candidate(" in receiver),
        "receive_length_capped_to_remaining":(
            "*DAT_00122ad4 - *DAT_00122ac4" in receiver and
            "if ((uint)param_2[9] <= uVar6)" in receiver),
        "receive_counter_advances_and_finishes":(
            "*puVar1 = uVar6;" in receiver and
            "*puVar1 < *puVar2" in receiver and
            "*puVar2 = 0;" in receiver),
        "boot_registers_callback_not_immediate_memory_write":(
            "mpf_fastboot_response(uVar1)" in boot and
            "DAT_00122978" in boot),
        "boot_callback_invokes_bootm_then_reset_if_returned":(
            "mpf_bootm_command(0,0,2,&local_14)" in completion and
            "mpf_reset_cpu_candidate(" in completion),
        "image_classifier_distinguishes_legacy_and_FDT":(
            "mpf_fdt_header_checker()" in classify and
            "uVar2 = 1" in classify and "uVar2 = 2" in classify),
        "bootm_legacy_header_and_data_crc_paths":(
            "mpf_legacy_header_crc(" in loader and
            "mpf_legacy_payload_crc(" in loader),
        "bootm_FDT_structured_image_path":"FUN_0010f8a4(" in loader,
    }
    return {
        "schema":SCHEMA,"verified_original_uboot_sha256":EXPECTED_SHA,
        "checks":checks,"all_static_evidence_passed":all(checks.values()),
        "ram_receive_base":RAM_BASE,
        "max_advertised_download_bytes":MAX_ADVERTISED,
        "mathematical_unchecked_end_address":RAM_BASE+MAX_ADVERTISED,
        "bootloader_downloads_do_not_imply_free_ram":True,
        "FIT_signature_required_proven":False,
        "raw_zImage_via_fastboot_boot_proven":False,
        "physical_fastboot_session_or_ram_execution_proven":False,
        "USB_authorization_state_known_at_boot":False,
        "safe_nonpersistent_kernel_initramfs_FDT_layout_proven":False,
        "physical_hardware_access":False,"vendor_bootloader_executed":False,
    }

def markdown(d):
    lines=[
        "# MPFree static U-Boot Fastboot RAM boot path","",
        "Download permission, address, length and bootm are static observations.","",
        "Original U-Boot SHA256: "+d["verified_original_uboot_sha256"],
        "","## Checks"]
    lines+=["- "+k+": "+str(v) for k,v in d["checks"].items()]
    lines+=["","## Limits","- No physical device or ROM executed.",
            "- Max advertised size is NOT safe RAM allocation.",
            "- Legacy/FDT classification does not prove FIT signature policy.",
            "- Confirm boot image type, FDT placement and authorization first.",""]
    return "\n".join(lines)
