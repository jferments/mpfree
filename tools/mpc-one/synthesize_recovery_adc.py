#!/usr/bin/env python3
"""Prove MPC One recovery-button ADC branch from private ARM and string xrefs.

Read-only synthesis of SHA-verified owner Ghidra outputs. Never executes
vendor code or accesses physical USB, ADC, eMMC, OTP, or MaskROM.
"""
from __future__ import annotations
import json

SCHEMA = "mpfree-recovery-adc-evidence-v1"
FILES = {
    "uboot": "static/v11/v11-ghidra-uboot.json",
    "strings": "static/wide/p3-uboot-ghidra.json",
    "env": "static/wide/storage-env-boot-dtb.json",
    "fdt": "static/wide/p1-p3-embedded-fdt-crossstage.json",
}
ERROR_MESSAGES = {
    "0x001010aa": "Recovery button ADC not found",
    "0x001010f0": "Failed to start recovery button ADC channel",
    "0x00101100": "Failed to read recovery button ADC channel",
}
ARM_SEQUENCE = {
    "0x001010a4": "bl 0x001147f0",
    "0x001010ea": "bl 0x00115ef4",
    "0x001010fa": "bl 0x00115f30",
    "0x00101106": "bl 0x00115ed8",
    "0x0010110a": "ldr r3,[sp,#0xc]",
    "0x0010110c": "cmp r3,#0x63",
    "0x0010110e": "ittt ls",
    "0x00101114": "strb.ls r2,[r3,#0x0]",
}


def norm(value):
    return "0x%08x" % int(str(value), 16)


def build(read):
    d = {k: json.loads(read(p)) for k, p in FILES.items()}
    u = d["uboot"]
    if u.get("role") != "uboot" or not u.get("source_sha256_prechecked_by_launcher"):
        raise ValueError("Missing verified U-Boot role/provenance")
    functions = u["analyzed_functions"]
    recovery = functions["0x00100fe4"]
    board = functions["0x001011e0"].get("pseudocode", "")
    getvar = functions["0x00100ef0"].get("pseudocode", "")
    arm = {norm(x["address"]): x["text"]
           for x in recovery["machine_instruction_evidence"]}
    arm_matches = {at: arm.get(at) == expected
                   for at, expected in ARM_SEQUENCE.items()}
    diagnostics = {}
    for address, message in ERROR_MESSAGES.items():
        hits = [s for s in d["strings"]["strings"]
                if s.get("text") == message and
                any(norm(x["from"]) == address for x in s.get("xrefs", []))]
        diagnostics[address] = len(hits) == 1
    dts = "\n".join(
        d["fdt"]["uboot_stage_fdt"][0]["selected_dts_lines"])
    adc_node = (
        dts.split("saradc@ff100000 {", 1)[1].split("};",1)[0]
        if "saradc@ff100000 {" in dts else "")
    env = d["env"]["p2"]["boot_related_variables"]
    slots = u.get("direct_boot_probe", {}).get(
        "reboot_mode_original_literal_slots", {})
    adc_pointer = slots.get("recovery_flag_pointer", {}).get("value")
    exported_pointer = slots.get(
        "board_setup_recovery_flag_pointer", {}).get("value")
    pointer_equal = (adc_pointer == exported_pointer
                     if adc_pointer and exported_pointer else None)
    checks = {
        "three_diagnostic_xrefs_prove_recovery_adc": all(diagnostics.values()),
        "original_uboot_fdt_saradc_enabled": all(
            item in adc_node for item in (
                'compatible = "rockchip,saradc";',
                'reg = <0xff100000 0x100>;',
                'status = "okay";'))
            and 'adc0 = "/saradc@ff100000";' in dts,
        "adc_driver_calls_and_unsigned_comparison_original_arm": all(arm_matches.values()),
        "recovery_adc_threshold_100_in_decompiler": (
            "if (local_1c[0] < 100)" in recovery["pseudocode"]
            and "*DAT_00101154 = 1;" in recovery["pseudocode"]),
        "fastboot_getvar_uses_same_class_10_device_channel_0": all(
            term in getvar for term in (
                "FUN_001147f0(10,0,&local_24)",
                "FUN_00115ef4(local_24,0)",
                "FUN_00115f30(local_24,0,&local_20)")),
        "board_setup_exports_flag_as_dorecovery": all(
            term in board for term in (
                "PTR_DAT_001013ac",
                "PTR_s_dorecovery_001013b4",
                "FUN_00108560(")),
        "saved_dorecovery_runs_stock_fastboot": (
            ("x$" + "{dorecovery} = x1") in env["az01_bootcmd"]
            and "fastboot 0" in env["az01_recovercmd"]),
        "reboot_register_and_magic_verified": (
            slots.get("mode_register_pointer", {}).get("value") ==
            "0xff730094"
            and slots.get("mode_prefix_constant", {}).get("value") ==
            "0x5242c300"),
        "recovery_adc_and_board_export_same_runtime_flag_pointer": pointer_equal,
    }
    return {
        "schema": SCHEMA,
        "source_uboot_sha256": u["source_sha256_prechecked_by_launcher"],
        "diagnostic_string_xrefs": diagnostics,
        "original_arm_evidence": arm_matches,
        "checks": checks,
        "saradc_base_from_original_uboot_fdt": "0xff100000",
        "adc_trigger_channel_argument": 1,
        "getvar_other_adc_channel_argument": 0,
        "adc_recovery_threshold_unsigned_max_inclusive": 99,
        "recovery_flag_pointer_comparison": {
            "adc_writer": adc_pointer, "board_env_reader": exported_pointer,
            "equal": pointer_equal,
        },
        "all_existing_cross_checks_passed": all(
            value is True for key, value in checks.items()
            if key != "recovery_adc_and_board_export_same_runtime_flag_pointer"),
        "all_pointer_checks_passed": pointer_equal is True,
        "physical_device_access": False,
        "vendor_code_executed": False,
        "maskrom_restore_proven": False,
        "limits": [
            "The physical button and ADC voltage were not measured.",
            "ROM MaskROM entry is not shown by stock U-Boot recovery.",
            "Missing new board pointer data remains unknown, not false.",
            "A getvar read-only handler is not evidence of live transport access.",
        ],
    }


def markdown(doc):
    lines = [
        "# MPFree stock U-Boot recovery-button ADC evidence", "",
        "SHA-verified original P3: " + doc["source_uboot_sha256"],
        "Recovery is selected when the unsigned channel-1 ADC reading <=99.",
        "Actual button levels and immutable MaskROM recovery remain unproven.",
        "", "## Evidence checks",
    ]
    lines.extend("- " + k + ": " + str(v)
                 for k, v in doc["checks"].items())
    lines.extend(["", "## Cross-stage pointers",
                  json.dumps(doc["recovery_flag_pointer_comparison"], indent=2),
                  "", "## Boundaries"])
    lines.extend("- " + x for x in doc["limits"])
    return "\n".join(lines) + "\n"
