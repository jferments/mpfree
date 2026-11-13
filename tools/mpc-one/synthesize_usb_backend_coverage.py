#!/usr/bin/env python3
"""Verify USB protocol selector's callback table against SHA-pinned ELF/Ghidra.

Read-only structural matching. "Present" means a static table slot points to
a recognized vendor function, never actual USB/MaskROM capability or execution.
"""
from __future__ import annotations
import json

SCHEMA="mpfree-usb-backend-callback-coverage-v1"
FILES={
    "tables":"static/v11/v11-raw-backend-tables-libaz01-prog.json",
    "usb":"usb-protocol-evidence.json",
    "lib":"static/v11/v11-ghidra-libaz01-prog.json",
}
SLOTS={
    "open":"+0x8",
    "write_bootloader":"+0x18",
    "write_partition":"+0x1c",
    "stream_partition":"+0x20",
    "commit_bootloader":"+0x2c",
    "reset":"+0x34",
}
EXPECTED={
    "ff/42/03":{"open","write_bootloader","write_partition",
                "stream_partition","commit_bootloader","reset"},
    "ff/61/7a":{"open","write_bootloader","write_partition",
                "stream_partition","reset"},
    "ff/06/05":{"open","write_bootloader","write_partition","reset"},
}


def build(read):
    docs={k:json.loads(read(path)) for k,path in FILES.items()}
    tables,usb,lib=(docs[key] for key in ("tables","usb","lib"))
    source_sha=lib.get("source_sha256_prechecked_by_launcher")
    if (not isinstance(source_sha,str) or len(source_sha)!=64
        or lib.get("role")!="libaz01-prog"
        or tables.get("source_sha256")!=source_sha
        or usb.get("verified_source_library_sha256")!=source_sha):
        raise ValueError("Source provenance mismatch across USB tables/ELF/Ghidra")
    base=int(lib["ghidra_image_base"],16)
    if base!=int(usb["ghidra_image_base"],16):
        raise ValueError("USB and Ghidra virtual address bias differ")
    all_tables=tables["tables"]
    outputs=[]
    for item in usb["verified_backend_interfaces"]:
        desc=item["usb_descriptor"]
        # V11 records USB fields as hexadecimal strings with 0x prefixes,
        # while the compact backend names omit prefixes. Normalize BOTH.
        # int(..., 16) accepts "ff"/"42"/"03" and "0xff"/"0x42"/"0x03".
        try:
            identity="/".join(
                format(int(desc[field], 16), "02x")
                for field in ("class", "subclass", "protocol")
            )
        except (KeyError, TypeError, ValueError) as exc:
            raise ValueError("Invalid USB descriptor field") from exc
        if identity not in EXPECTED:
            raise ValueError("Unknown USB descriptor family "+identity)
        matches=[x for x in all_tables
                 if x["base_vaddr"]==item["candidate_table_vaddr"]]
        if len(matches)!=1 or not item["initializer_references_table"]:
            raise ValueError("Missing source-linked unique backend "+identity)
        table=matches[0]
        ghidra_evidence={
            x["offset"]:x for x in item["backend_callback_evidence"]}
        actual=set(table["callbacks"])
        checks={}
        detail=[]
        for name,slot in SLOTS.items():
            callback=table["callbacks"].get(name)
            evidence=ghidra_evidence.get(slot)
            if evidence is None:
                raise ValueError("Incomplete callback slot evidence "+identity+"/"+slot)
            if callback is None:
                checks[name]=evidence.get("resolved_function") is None
            else:
                expected_ghidra="0x%08x" % (int(callback,16)+base)
                checks[name]=(
                    evidence.get("resolved_function")==expected_ghidra
                    and evidence.get("ghidra_function_recognized") is True
                    and expected_ghidra in lib["analyzed_functions"])
            detail.append({
                "operation":name,
                "slot":slot,
                "present_in_original_table":callback is not None,
                "matched_library_function":(
                    evidence.get("resolved_function") if callback else None),
                "static_crosscheck_passed":checks[name],
                "physical_operation_tested":False,
            })
        outputs.append({
            "descriptor":identity,
            "protocol_interpretation":item["upstream_protocol_family"],
            "table_vaddr":table["base_vaddr"],
            "constructor_vaddr":item["initializer_vaddr"],
            "callbacks":detail,
            "present_operations":sorted(actual),
            "missing_operations":sorted(set(SLOTS)-actual),
            "table_and_ghidra_callback_matches":all(checks.values()),
            "expected_ops_match_original":actual==EXPECTED[identity],
            "live_device_stage_or_transport_proven":False,
        })
    if len(outputs)!=3 or {x["descriptor"] for x in outputs}!=set(EXPECTED):
        raise ValueError("Incomplete set of three USB transport backends")
    return {
        "schema":SCHEMA,
        "original_library_sha256":source_sha,
        "ghidra_elf_address_bias":"0x%08x" % base,
        "backends":outputs,
        "all_source_only_backends_matched":all(
            b["table_and_ghidra_callback_matches"]
            and b["expected_ops_match_original"] for b in outputs),
        "differences_are_table_slots_not_physical_capability":True,
        "physical_usb_executed":False,
        "real_vendor_usb_protocol_verified":False,
        "silicon_maskrom_state_proven":False,
        "whole_emmc_recovery_proven":False,
    }


def markdown(data):
    lines=["# MPFree original three USB backend callback coverage","",
           "Verified ELF table + Ghidra callbacks; vendor code not run.",""]
    for b in data["backends"]:
        lines+=["## "+b["descriptor"],"","- "+b["protocol_interpretation"],
                "- Present slots: "+", ".join(b["present_operations"]),
                "- Missing slots: "+", ".join(b["missing_operations"]),
                "- All static callback addresses match: "+
                str(b["table_and_ghidra_callback_matches"]),"",]
    lines += [
        "**Limit:** a null backend callback slot is not proof that silicon",
        "cannot perform the same operation by another path.",
        "**Limit:** no actual original updater, USB or MaskROM device was run.",""]
    return "\n".join(lines)
