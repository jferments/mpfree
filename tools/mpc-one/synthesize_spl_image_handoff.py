#!/usr/bin/env python3
"""Original P1 SPL image-header-to-indirect-handoff cross-check.

Reads only private ZIP-indexed Ghidra evidence. Interprets the 64-byte
legacy image header layout; actual ROM/SPL code and storage are never run.
"""
from __future__ import annotations

import json

SCHEMA = "mpfree-spl-image-header-handoff-v1"
INPUTS = {
    "spl": "static/v11/v11-ghidra-spl.json",
    "p1": "static/wide/p1-sram-ghidra.json",
    "factory_p3": "static/wide/storage-env-boot-dtb.json",
    "uboot": "static/v11/v11-ghidra-uboot.json",
}
# Standard legacy U-Boot image header (big-endian 32-bit fields).
HEADER_WORD_OFFSETS = {"size": 0x0c, "load": 0x10, "entry": 0x14}
EXPECTED_WRITE_SITES = (
    "0xff7052a8", "0xff7052dc", "0xff7052f2",
)
EXPECTED_CALLBACK_SLOT = "0xff70a250"
EXPECTED_CONTEXT = "0xff70a244"


def build(read):
    docs = {role: json.loads(read(path)) for role, path in INPUTS.items()}
    spl, p1 = docs["spl"], docs["p1"]
    factory_p3 = docs["factory_p3"]["p3"]
    uboot = docs["uboot"]
    source = spl.get("source_sha256_prechecked_by_launcher")
    if (spl.get("role") != "spl" or not isinstance(source, str)
            or len(source) != 64 or p1.get("sha256") != source):
        raise ValueError("SPL/P1 SHA provenance mismatch")
    if (not factory_p3.get("valid") or
        not factory_p3.get("header_crc32_valid") or
        not factory_p3.get("payload_crc32_valid") or
        factory_p3.get("sha256") !=
            uboot.get("source_sha256_prechecked_by_launcher") or
        uboot.get("role") != "uboot"):
        raise ValueError("Factory P3 header and original U-Boot SHA not matched")
    funcs = spl["analyzed_functions"]
    def code(addr):
        return funcs.get(addr, {}).get("pseudocode", "")
    parser = code("0xff7052c0")
    fallback = code("0xff70529c")
    loader = code("0xff7054a4")
    dispatch = code("0xff705310")
    loop = code("0xff705350")
    context = spl["direct_boot_probe"]["extended_stage_evidence"]
    original = context.get("legacy_image_header_original_words", {})
    xrefs = context.get("second_handoff_pointer_slot_static_xrefs", [])
    refs = {row.get("from", "").lower()
            for row in xrefs if row.get("type") == "WRITE"}
    magic_hits = p1.get("literal_hits", {}).get("legacy_uimage_magic", [])
    checked = {
        "source_image_contains_legacy_header_magic": any(
            row.get("address", "").lower() == "0xff705308"
            for row in magic_hits),
        "header_parser_checks_image_magic": (
            "*param_1 == DAT_ff705308" in parser),
        "header_load_word_offset_0x10": "param_1[4]" in parser,
        "header_entry_word_offset_0x14": "param_1[5]" in parser,
        "header_size_word_offset_0x0c": "param_1[3]" in parser,
        "header_parser_byte_swaps_network_endian_fields": all(
            text in parser for text in (
                "uVar3 << 0x18", "uVar3 >> 0x18")),
        "entry_field_populates_context_word_3": (
            "piVar2[3] = uVar3 << 0x18" in parser),
        "load_field_populates_context_word_3_alternate": (
            "DAT_ff70530c[3] = uVar3;" in parser),
        "default_handler_sets_context_entry_0x100000": (
            "puVar1[3] = 0x100000;" in fallback
            and "DAT_ff7052b8" in fallback),
        "media_loader_passes_buffer_to_header_parser": (
            "FUN_ff7052c0(DAT_ff705598)" in loader),
        "boot_selector_calls_context_based_handoff": (
            "FUN_ff705310(DAT_ff70540c)" in loop),
        "handoff_indirect_branch_uses_context_plus_0x0c": (
            "(**(code **)(param_1 + 0xc))()" in dispatch),
        "three_original_instruction_writes_to_handoff_slot": all(
            site in refs for site in EXPECTED_WRITE_SITES),
        "verified_context_plus_0x0c_equals_callback_slot": (
            context.get("indirect_handoff_context_address") == EXPECTED_CONTEXT
            and context.get("indirect_handoff_function_pointer_slot")
                == EXPECTED_CALLBACK_SLOT),
        "original_callback_slot_zero_before_loading": (
            context.get("indirect_handoff_original_word") == "0x00000000"),
        "original_factory_p3_image_loads_at_0x00100000": (
            factory_p3.get("load_address") == "0x00100000"),
        "original_factory_p3_entry_header_field_is_zero": (
            factory_p3.get("entry_address") == "0x00000000"),
        "original_factory_p3_is_legacy_type5_arm": (
            factory_p3.get("legacy_type_id") == 5 and
            factory_p3.get("arch_id") == 2),
    }
    raw_magic = original.get("image_header_magic_literal_word")
    context_ptr = original.get("image_header_context_ptr")
    default_context_ptr = original.get("image_header_default_context_ptr")
    checks_new = {
        "literal_header_context_is_same_as_handoff_context": (
            context_ptr == EXPECTED_CONTEXT
            if context_ptr is not None else None),
        "default_image_handler_uses_same_context": (
            default_context_ptr == EXPECTED_CONTEXT
            if default_context_ptr is not None else None),
        "image_header_magic_word_consistent_with_legacy_magic": (
            raw_magic in ("0x27051956", "0x56190527")
            if raw_magic is not None else None),
    }
    # Do not silently mark unavailable new literals as proven.
    physical = {
        "rom_execution_or_maskrom_accepted": False,
        "original_p3_image_loaded_on_hardware": False,
        "physical_entrypoint_memory_safe": False,
        "custom_image_signature_or_boot_policy_accepted": False,
        "full_emmc_recovery_proven": False,
    }
    return {
        "schema": SCHEMA,
        "source_spl_sha256": source,
        "checks": checked,
        "new_literal_pointer_checks": checks_new,
        "all_prior_static_evidence_passed": all(checked.values()),
        "all_new_literal_checks_passed": all(
            value is True for value in checks_new.values()),
        "header_offsets_bytes": HEADER_WORD_OFFSETS,
        "SPL_original_callback_slot": EXPECTED_CALLBACK_SLOT,
        "SPL_original_callback_word": context.get(
            "indirect_handoff_original_word"),
        "SPL_entry_candidate_source": (
            "legacy header entry offset +0x14 (flag-dependent "
            "alternate header load field offset +0x10)"),
        "SPL_entry_address_runtime_value_known": False,
        "factory_P3_verified_legacy_header": {
            "image_type_id": factory_p3["legacy_type_id"],
            "load_address": factory_p3["load_address"],
            "entry_address": factory_p3["entry_address"],
            "crc32_header_valid": factory_p3["header_crc32_valid"],
            "crc32_payload_valid": factory_p3["payload_crc32_valid"],
        },
        "factory_P3_zero_entry_requires_load_branch_or_other_adjustment": True,
        "physical_proofs": physical,
        "notes": [
            "Header offset relationship, unlike a fixed callback pointer, "
            "explains why original slot bytes begin zero.",
            "Header handler can use entry, load or default 0x100000 "
            "depending on source/header and context flags.",
            "Candidate entry execution remains subject to loader checks, "
            "RAM placement and any unobserved authentication policy.",
            "Ghidra data xrefs are static references, not live writes.",
        ],
    }


def markdown(d):
    rows = ["# Original SPL image header to second-stage handoff", "",
            "Offline Ghidra/legacy-uImage layout evidence; no vendor execution.",
            "", "## Checks"]
    rows += ["- " + k + ": " + str(v) for k, v in d["checks"].items()]
    rows += ["", "## New literal pointer verification"]
    rows += ["- " + k + ": " + str(v)
             for k, v in d["new_literal_pointer_checks"].items()]
    rows += ["", "The +0x0c context word is called by FF705310. "
             "FF7052C0 loads load/entry fields from the image header; "
             "FF70529C supplies a 0x100000 default.",
             "This is static evidence, not a ROM-accepted or safe install.", ""]
    return "\n".join(rows)
