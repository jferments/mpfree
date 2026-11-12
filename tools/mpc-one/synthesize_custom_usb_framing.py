#!/usr/bin/env python3
"""MPFree FF/61/7A backend framing/USB URB offline evidence synthesis.

Consumes ONLY the verified owner-private V11 Ghidra JSON; emits a bounded
summary without copying vendor pseudocode. Never executes vendor binaries,
opens devices or changes actual storage. This protocol remains UNIDENTIFIED.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re
import stat

SOURCE_SHA = "af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623"
SCHEMA = "mpfree-ff617a-offline-framing-v1"
TABLE = "0x00012b48"
CTOR = "0x0001b49c"
ROLES = {
    "0x0001b49c": "interface constructor: installs callback table",
    "0x0001a170": "backend open callback",
    "0x0001b35c": "one of the backend transfer callbacks",
    "0x0001b27c": "one of the backend transfer callbacks",
    "0x0001b150": "streaming callback using application-provided data",
    "0x0001a8d0": "control-message construction",
    "0x0001a9bc": "string-labelled command construction",
    "0x0001aabc": "framed command construction and validation",
    "0x0001a178": "chunked USB OUT transfer helper",
    "0x0001a674": "framed command transfer and response handling",
    "0x0001a314": "framed response matching and flags",
    "0x0001afd4": "framed completion read/validation",
    "0x0001d644": "USB device filesystem asynchronous URB transport",
}

MAGIC = re.compile(r"\b0x(?:5aa5|a55a)[0-9a-f]{4}\b", re.I)
USB_IOCTL = re.compile(r"\b0x(?:802c550a|4004550d)\b", re.I)
SOURCE_FINDER = re.compile(r"\b(?:0x100000|300000|5000|0x18|0x20)\b")
IOC_KNOWN = {
    0x802c550a: "USBDEVFS_SUBMITURB",
    0x4004550d: "USBDEVFS_REAPURBNDELAY",
}


def ioctl_fields(value):
    """Decode Linux generic ioctl fields independently of host architecture."""
    return {
        "raw": f"0x{value:08x}", "direction": (value >> 30) & 3,
        "size_bytes": (value >> 16) & 0x3fff,
        "type": f"0x{(value >> 8) & 0xff:02x}",
        "command_number": value & 0xff,
    }


def validated_report(document, report_sha):
    if document.get("schema") != "mpfree-v11-backend-selector-and-boot-recovery-static":
        raise ValueError("Unexpected Ghidra V11 report schema")
    if document.get("role") != "libaz01-prog":
        raise ValueError("Unexpected V11 Ghidra role")
    if document.get("source_sha256_prechecked_by_launcher") != SOURCE_SHA:
        raise ValueError("Unrecognized original updater-library SHA-256")
    tables = {
        t.get("base_elf_vaddr"): t
        for t in document.get("verified_raw_table_candidates", [])
    }
    table = tables.get(TABLE)
    if not table or CTOR not in table.get("xref_source_functions", []):
        raise ValueError("Verified FF/61/7A callback table constructor xref missing")
    functions = document.get("analyzed_functions", {})
    if not isinstance(functions, dict):
        raise ValueError("Ghidra analyzed functions must be a map")
    members = []
    for slot in table.get("observed_ghidra_slots", []):
        if slot.get("relative") not in (8, 0x18, 0x1c, 0x20, 0x2c, 0x34):
            continue
        raw_address = slot.get("resolved_function")
        address = ("0x%08x" % int(str(raw_address), 16)) if raw_address else None
        body = functions.get(address, {}) if address else {}
        members.append({
            "slot": f"+0x{slot['relative']:x}",
            "function": address,
            "decompiled": "pseudocode" in body,
            "callees": body.get("graph", {}).get("callees", []),
            "computed_call_sites": body.get("graph", {}).get("computed", []),
        })
    analyzed = []
    magic_sites = {}
    ioctl_sites = {}
    for address, role in ROLES.items():
        rec = functions.get(address, {})
        body = rec.get("pseudocode", "")
        hexvalues = sorted(set(x.lower() for x in MAGIC.findall(body)))
        ioctls = sorted(set(x.lower() for x in USB_IOCTL.findall(body)))
        if hexvalues:
            magic_sites[address] = hexvalues
        if ioctls:
            ioctl_sites[address] = ioctls
        analyzed.append({
            "address": address, "analysis_role": role,
            "decompiled": bool(body),
            "function_size_ghidra": rec.get("size"),
            "decompilation_sha256": hashlib.sha256(body.encode()).hexdigest() if body else None,
            "direct_callees": rec.get("graph", {}).get("callees", []),
            "computed_calls": rec.get("graph", {}).get("computed", []),
            "observed_magic": hexvalues,
            "observed_usb_ioctls": ioctls,
            "observed_transfer_values": sorted(set(SOURCE_FINDER.findall(body))),
        })
    ioc = []
    for raw, label in IOC_KNOWN.items():
        encoded = f"0x{raw:08x}"
        ioc.append({
            "ioctl": encoded, "linux_usbdevfs_name": label,
            "fields": ioctl_fields(raw),
            "seen_in_verified_pseudocode": any(
                encoded in values for values in ioctl_sites.values()),
        })
    compared_ids = [f"0x5aa5{x:04x}" for x in
                    (0xfe01, 0xfd02, 0xfc03, 0xfb04, 0xfa05, 0xf906)]
    constructed_values = ["0xa55a0101", "0xa55a0202"]
    expected_magics = compared_ids + constructed_values
    seen = set(x for values in magic_sites.values() for x in values)
    # Equality comparisons and outbound construction are DISTINCT kinds
    # of evidence. The 0x5aa5fc04 value is only a range bound.
    comparison_only = ["0x5aa5fc04"] if "0x5aa5fc04" in seen else []
    confirmed_ids = sorted(x for x in compared_ids if
                           x in magic_sites.get("0x0001a674", []) and
                           x in magic_sites.get("0x0001a314", []))
    observed_constructed = sorted(
        x for x in constructed_values
        if x in magic_sites.get("0x0001a8d0", []))
    response = functions.get("0x0001a314", {}).get("pseudocode", "")
    response_checks = {
        "min_24_byte_frame": "local_30 < 0x18" in response,
        "declared_length_checked": "local_30 < *(ushort *)(param_4 + 0x16)" in response,
        "message_identifier_matches_request": (
            "*(int *)(param_4 + 4) != *(int *)(param_2 + 4)" in response),
        "correlation_field_matches_request": (
            "*(int *)(param_4 + 0x10) != *(int *)(param_2 + 0x10)" in response),
        "completion_flag_bit0_required": "(uVar1 & 1) == 0" in response,
        "error_flag_bit1_branch": "(uVar1 & 2) == 0" in response,
        "error_status_word_at_plus_18": "*(int *)(param_4 + 0x18)" in response,
        "negative_error_values_observed": "-0xb" in response and "-9" in response,
    }
    signature_warnings = []
    for address in ("0x0001a674", "0x0001d644", "0x0001aabc"):
        pseudocode = functions.get(address, {}).get("pseudocode", "").strip()
        if pseudocode.startswith("void "):
            signature_warnings.append({
                "address": address,
                "reason": "Ghidra inferred void return; callers may use a returned status. Verify ARM callsite/register conventions.",
            })
    return {
        "schema": SCHEMA,
        "source_ghidra_report_sha256": report_sha,
        "verified_vendor_elf_sha256": SOURCE_SHA,
        "interface": {"class": "0xff", "subclass": "0x61", "protocol": "0x7a"},
        "protocol_family": "UNIDENTIFIED; framed proprietary/vendor-specific candidate",
        "callback_table": TABLE,
        "constructor": CTOR,
        "constructor_table_xref_verified": True,
        "callback_evidence": members,
        "focused_function_evidence": analyzed,
        "magic_evidence": {
            "code_points": {value: [a for a, v in magic_sites.items() if value in v]
                            for value in sorted(seen)},
            "eight_known_values_present": all(x in seen for x in expected_magics),
            "equality_matched_message_identifiers": confirmed_ids,
            "constructed_outgoing_control_values": observed_constructed,
            "comparison_only_range_bounds": comparison_only,
            "unobserved_expected_values": [x for x in expected_magics if x not in seen],
        },
        "linux_usbdevfs_ioctls": ioc,
        "response_validation_checks": response_checks,
        "decompiler_return_type_warnings": signature_warnings,
        "tentative_frame_fields": [
            {"offset": "context+0x20", "meaning": "incremented 16-bit sequence counter",
             "evidence_functions": ["0x0001a8d0", "0x0001a9bc", "0x0001b35c"]},
            {"offset": "frame+0x04", "meaning": "message identifier used in send/response comparison",
             "evidence_functions": ["0x0001a674", "0x0001a314"]},
            {"offset": "frame+0x10", "meaning": "field compared against expected response value",
             "evidence_functions": ["0x0001a314"]},
            {"offset": "frame+0x14", "meaning": "response flags used for accept/reject",
             "evidence_functions": ["0x0001a314"]},
            {"offset": "frame+0x16", "meaning": "declared message length",
             "evidence_functions": ["0x0001a674", "0x0001a314"]},
        ],
        "safety_boundaries": [
            "Static compiler/decompiler output only: behavior unverified on a physical USB device.",
            "This code does not execute Akai firmware, send USB traffic or open /dev.",
            "The IOC fields describe Linux host syscalls, not protocol opcode IDs.",
            "Response field interpretations are hypotheses until verified against machine code and runtime.",
            "Neither backend identity nor true silicon MaskROM can be established from these markers.",
        ],
    }


def render_markdown(rep):
    output = [
        "# FF/61/7A USB backend — owner-private static framing evidence",
        "",
        "## Confidence and limitations",
        "Vendor framing appears structured and stateful. Its published protocol",
        "name and actual physical boot-stage identity are **NOT established**.",
        "This document does not contain proprietary function source.",
        "",
        "## Verified metadata",
        "- Library SHA-256: " + rep["verified_vendor_elf_sha256"],
        "- Constructor and callback table: " + rep["constructor"] +
        " / " + rep["callback_table"],
        "- Verified known magic values: " +
        str(rep["magic_evidence"]["eight_known_values_present"]),
        "",
        "## Observed 32-bit message identifiers",
    ]
    for value in rep["magic_evidence"]["equality_matched_message_identifiers"]:
        addresses = rep["magic_evidence"]["code_points"][value]
        output.append("- Equality-tested: " + value + " in " + ", ".join(addresses))
    for value in rep["magic_evidence"]["constructed_outgoing_control_values"]:
        output.append("- Constructed outgoing value (NOT equality-tested): " + value)
    for value in rep["magic_evidence"]["comparison_only_range_bounds"]:
        output.append("- Range-bound only; NOT established as an opcode: " + value)
    output += ["", "## Response validation predicates (decompilation evidence)"]
    for label, observed in rep["response_validation_checks"].items():
        output.append("- " + label + ": " + str(observed))
    output += ["", "## Decompiled return-type ambiguities"]
    for warn in rep["decompiler_return_type_warnings"]:
        output.append("- " + warn["address"] + ": " + warn["reason"])
    output += ["", "## Linux USB host transfer operations"]
    for item in rep["linux_usbdevfs_ioctls"]:
        output.append("- " + item["ioctl"] + ": " + item["linux_usbdevfs_name"] +
                      " (seen=" + str(item["seen_in_verified_pseudocode"]) + ")")
    output += ["", "## Callback table slots"]
    for item in rep["callback_evidence"]:
        output.append("- " + item["slot"] + " -> " + str(item["function"]) +
                      ", decompiled=" + str(item["decompiled"]))
    output += ["", "## Candidate frame fields"]
    for field in rep["tentative_frame_fields"]:
        output.append("- " + field["offset"] + ": " + field["meaning"])
    output += ["", "## Immediate research questions",
               "- What are the real roles of every message type and status flag?",
               "- How do sequence numbers and errors alter framing and retransmission?",
               "- Is the frame header exactly 24 bytes for all commands?",
               "- What USB interface devices, if any, expose FF/61/7A physically?",
               "- How does the 1 MiB transfer chunk flow through command callbacks?",
               "- Which paths are read-only versus state-changing?",
               "",
               "No live USB, firmware update or physical MPC state was touched.",
               ""]
    return "\n".join(output)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ghidra-json", required=True, type=Path)
    ap.add_argument("--out-dir", required=True, type=Path)
    a = ap.parse_args()
    src, dst = a.ghidra_json.expanduser(), a.out_dir.expanduser()
    if src.is_symlink() or not src.is_file() or not stat.S_ISREG(src.lstat().st_mode):
        ap.error("Ghidra JSON must be an existing regular file, never device/symlink")
    if dst.is_symlink() or not dst.is_dir():
        ap.error("Output directory must exist and must not be a symlink")
    raw = src.read_bytes()
    report = validated_report(json.loads(raw), hashlib.sha256(raw).hexdigest())
    (dst / "usb-ff617a-framing.json").write_text(json.dumps(report, indent=2) + "\n")
    (dst / "usb-ff617a-framing.md").write_text(render_markdown(report) + "\n")
    print("FF/61/7A static framing: ", len(report["callback_evidence"]),
          "callback slots,", len(report["magic_evidence"]["code_points"]),
          "observed magic words,", len(report["linux_usbdevfs_ioctls"]),
          "host USB IOCTL families")


if __name__ == "__main__":
    main()
