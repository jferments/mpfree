#!/usr/bin/env python3
"""Produce a focused PRIVATE evidence map from V11 USB backend Ghidra output.

Input: the V11 JSON generated from the owner's exact verified 3.10 updater
library. No binary execution, decompiler invocation, USB access or MPC access.
Output: SHA-backed structured evidence, no proprietary pseudocode copied.
Labels from upstream Fastboot/RockUSB source identify interface compatibility,
NOT a physical target's BootROM, firmware state, or authorization.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re

SCHEMA = "mpfree-usb-protocol-evidence-v1"
SOURCE_SHA = "af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623"
INTERFACES = (
    {
        "table": "0x00012a74", "ctor": "0x00016ff8",
        "class": "0xff", "subclass": "0x42", "protocol": "0x03",
        "protocol_family": "Android Fastboot-compatible",
        "upstream": "https://android.googlesource.com/platform/system/core/+/refs/heads/main/fastboot/fastboot.cpp",
        "marker_functions": {"0x00015bec": "0x59414b4f", "0x00016028": "0x4c494146"},
        "marker_interpretation": "Fastboot OKAY and FAIL response words, little-endian.",
    },
    {
        "table": "0x00012b48", "ctor": "0x0001b49c",
        "class": "0xff", "subclass": "0x61", "protocol": "0x7a",
        "protocol_family": "Unknown vendor-specific",
        "upstream": None,
        "marker_functions": {},
        "marker_interpretation": "No independently validated protocol identification.",
    },
    {
        "table": "0x00012bd8", "ctor": "0x0001ca80",
        "class": "0xff", "subclass": "0x06", "protocol": "0x05",
        "protocol_family": "Rockchip RockUSB-compatible",
        "upstream": "https://github.com/rockchip-linux/u-boot/blob/next-dev/drivers/usb/gadget/f_rockusb.c",
        "marker_functions": {"0x0001c73c": "0x43425355", "0x0001c8c8": "0x43425355"},
        "marker_interpretation": "USB Bulk-Only Transport command block (USBC) signature.",
    },
)


def regular_file(path: Path):
    if path.is_symlink() or not path.is_file():
        raise ValueError("Expected existing regular file, not symlink: " + str(path))
    return path


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def summarize(document, file_sha):
    if document.get("schema") != "mpfree-v11-backend-selector-and-boot-recovery-static":
        raise ValueError("Wrong V11 Ghidra report schema")
    if document.get("role") != "libaz01-prog":
        raise ValueError("Must use updater library report, not updater executable")
    if document.get("source_sha256_prechecked_by_launcher") != SOURCE_SHA:
        raise ValueError("V11 ELF hash does not match verified 3.10 library")
    tables = {t["base_elf_vaddr"]: t for t in document.get("verified_raw_table_candidates", [])}
    functions = document.get("analyzed_functions", {})
    results = []
    for item in INTERFACES:
        table = tables.get(item["table"])
        if table is None:
            raise ValueError("Missing candidate table: " + item["table"])
        members = []
        for slot in table.get("observed_ghidra_slots", []):
            rel = slot.get("relative")
            if rel not in (8, 0x18, 0x1c, 0x20, 0x2c, 0x34):
                continue
            fn = slot.get("resolved_function")
            if fn:
                normalized = "0x%08x" % int(str(fn), 16)
            else:
                normalized = None
            details = functions.get(normalized, {}) if normalized else {}
            body = details.get("pseudocode", "")
            members.append({
                "offset": "+0x%x" % rel,
                "resolved_function": normalized,
                "ghidra_function_recognized": bool(details),
                "decompilation_present": bool(body),
                "direct_callees": details.get("graph", {}).get("callees", []),
                "computed_call_count": len(details.get("graph", {}).get("computed", [])),
                "ghidra_body_size": details.get("size"),
            })
        markers = []
        for where, code in item["marker_functions"].items():
            body = functions.get(where, {}).get("pseudocode", "")
            markers.append({"function": where, "word": code,
                            "seen_in_decompilation": code in body if body else False})
        ctor_xref = item["ctor"] in table.get("xref_source_functions", [])
        results.append({
            "usb_descriptor": {k: item[k] for k in ("class", "subclass", "protocol")},
            "upstream_protocol_family": item["protocol_family"],
            "protocol_reference": item["upstream"],
            "candidate_table_vaddr": item["table"],
            "initializer_vaddr": item["ctor"],
            "initializer_references_table": ctor_xref,
            "marker_evidence": markers,
            "marker_interpretation": item["marker_interpretation"],
            "backend_callback_evidence": members,
            "not_proven": [
                "That this interface is physically present on owner's MPC",
                "That Linux, U-Boot or the immutable ROM is currently serving it",
                "That callback operations can safely run against real storage",
                "That static callback offset names prove the target's behavior",
            ],
        })
    warnings = [
        "External Fastboot/RockUSB interface matches identify protocol family, NOT silicon MaskROM state.",
        "The proprietary Ghidra report contains decompiled source. Keep its ZIP and this report private.",
        "Candidate tables overlap; only xref-linked constructor tables are treated as selected.",
        "No original updater library code is executed in QEMU by these tests.",
    ]
    return {
        "schema": SCHEMA,
        "input_ghidra_json_sha256": file_sha,
        "verified_source_library_sha256": SOURCE_SHA,
        "ghidra_image_base": document.get("ghidra_image_base"),
        "verified_backend_interfaces": results,
        "warnings": warnings,
        "follow_up_functions": [
            "0x00014fc4", "0x000152b0", "0x000153ac",
            "0x0001a178", "0x0001a674", "0x0001aabc", "0x0001afd4",
            "0x0001be1c", "0x0001c2bc", "0x0001d644",
        ],
        "questions": [
            "Does the FF/61/7A transport use another vendor-defined frame or loader protocol?",
            "Can we connect each backend's send/receive wrapper to actual framing and status checks?",
            "Which callbacks are read-only versus firmware-mutating, based on provable control flow?",
            "How does Fastboot 'OKAY'/'FAIL' correlate with earlier U-Boot permission gates?",
            "Can the RK3288 genuine immutable ROM enumerated state be independently demonstrated?",
        ],
    }


def render_markdown(report):
    rows = [
        "# MPFree USB protocol evidence map — offline static only",
        "",
        "The classifications below use V11 Ghidra function evidence and upstream",
        "reference source. They DO NOT prove which runtime boot stage is active.",
        "",
        "| Descriptor | Constructor | Selected table | Protocol family |",
        "| --- | --- | --- | --- |",
    ]
    for entry in report["verified_backend_interfaces"]:
        d = entry["usb_descriptor"]
        rows.append("| " + "/".join(d.values()) + " | " +
                    entry["initializer_vaddr"] + " | " +
                    entry["candidate_table_vaddr"] + " | " +
                    entry["upstream_protocol_family"] + " |")
    rows += ["", "## Callback resolution"]
    for entry in report["verified_backend_interfaces"]:
        rows.append("### " + entry["candidate_table_vaddr"])
        rows.append("- Constructor xref confirmed in Ghidra: " +
                    str(entry["initializer_references_table"]))
        for item in entry["backend_callback_evidence"]:
            rows.append("- " + item["offset"] + " -> " +
                        str(item["resolved_function"]) +
                        " (decompiled=" + str(item["decompilation_present"]) +
                        ", direct callees=" + str(len(item["direct_callees"])) + ")")
        for mark in entry["marker_evidence"]:
            rows.append("- Signature " + mark["word"] + " in " + mark["function"] +
                        ": " + str(mark["seen_in_decompilation"]))
    rows += ["", "## Verification limits"]
    rows.extend("- " + w for w in report["warnings"])
    rows += ["", "## Next research questions"]
    rows.extend("- " + q for q in report["questions"])
    return "\n".join(rows) + "\n"


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--ghidra-json", required=True, type=Path)
    p.add_argument("--out-dir", required=True, type=Path)
    a = p.parse_args()
    src = regular_file(a.ghidra_json)
    out = a.out_dir
    if out.is_symlink() or not out.is_dir():
        p.error("Output must be an existing real report directory")
    data = src.read_bytes()
    report = summarize(json.loads(data), sha256_bytes(data))
    (out / "usb-protocol-evidence.json").write_text(json.dumps(report, indent=2) + "\n")
    (out / "usb-protocol-evidence.md").write_text(render_markdown(report))
    print("MPFree USB protocol map:", len(report["verified_backend_interfaces"]),
          "interfaces, private reports in", out)


if __name__ == "__main__":
    main()
