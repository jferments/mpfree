#!/usr/bin/env python3
"""Offline ARM return-contract evidence from a private MPFree integrated ZIP.

Never executes firmware, invokes Ghidra/QEMU or opens USB/eMMC devices. The
owner's original archive stays private. The report contains only carefully
bounded derived metadata and small callsite excerpts, not full pseudocode.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import zipfile

INDEX = "ZIP_INDEX.json"
LIB = "static/v11/v11-ghidra-libaz01-prog.json"
FRAMING = "usb-ff617a-framing.json"
EXPECTED_LIBRARY_SHA = "af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623"
TARGETS = {
    "0x0001d644": "Linux usbdevfs URB transfer helper",
    "0x0001a674": "Framed request/response helper",
    "0x0001a314": "Framed response parser",
    "0x0001aabc": "Framed command builder",
    "0x0001b35c": "Backend callback calling framed transport",
}
CALL = re.compile(r"^bl(?:x)?\s+(0x[0-9a-f]+)$", re.I)
SET_R0 = re.compile(r"^(mov|mvn)\s+r0,#(0x[0-9a-f]+|\d+)$", re.I)
CHECK_R0 = re.compile(r"^(cmp|cmn|tst|teq)\s+r0,", re.I)
SUBS_R0_ZERO = re.compile(r"^subs\s+(r(?:1[0-2]|[0-9])),r0,#(?:0x0|0)$", re.I)
COPY_R0 = re.compile(r"^(?:cpy|mov)\s+(r(?:1[0-2]|[0-9])),r0$", re.I)
CONDITIONAL_BRANCH = re.compile(
    r"^b(?:ne|eq|lt|le|ge|gt|hi|ls|cs|cc|mi|pl|vs|vc)\s", re.I
)


def validate_index(archive: zipfile.ZipFile) -> dict:
    index = json.loads(archive.read(INDEX))
    if index.get("schema") != "mpfree-bootchain-integrated-zip-index-v1":
        raise ValueError("Unrecognized ZIP index schema")
    files = index.get("files")
    if not isinstance(files, list) or not files:
        raise ValueError("Missing indexed file list")
    names = archive.namelist()
    if len(names) != len(set(names)) or set(names) != {entry["name"] for entry in files} | {INDEX}:
        raise ValueError("ZIP member list does not match its index")
    if len(files) > 250 or sum(archive.getinfo(e["name"]).file_size for e in files) > 384*1024*1024:
        raise ValueError("Unexpectedly large audit archive")
    for entry in files:
        name = entry["name"]
        if name.startswith("/") or any(p in ("..", "") for p in name.split("/")):
            raise ValueError("Unsafe ZIP member name")
        blob = archive.read(name)
        if len(blob) != entry["bytes"] or hashlib.sha256(blob).hexdigest() != entry["sha256"]:
            raise ValueError("ZIP index hash/size mismatch: " + name)
    return {"indexed_members_verified": len(files), "crc_passed": archive.testzip() is None}


def normalize_addr(value: str) -> str:
    return f"0x{int(str(value), 16):08x}"


def instructions_for(function: dict) -> list[dict]:
    out = []
    for ins in function.get("machine_instruction_evidence", []):
        if not isinstance(ins, dict) or "address" not in ins or "text" not in ins:
            continue
        out.append({"address": normalize_addr(ins["address"]),
                    "text": str(ins["text"]).strip().lower()})
    return out


def normal_fallthrough_return_preserves_r0(instructions: list[dict], call_index: int) -> dict | None:
    """Conservative straight-line, normal-return-path observation only.

    A conditional branch can leave the path (e.g. canary failure); this
    verifies ONLY the fallthrough path. Do not confuse with all-path
    return-value propagation. Reject intervening calls, unconditional
    jumps, and instructions writing r0.
    """
    for distance, ins in enumerate(instructions[call_index + 1:call_index + 49], 1):
        text = ins["text"]
        if re.match(r"^ldr\\s+pc,\\[sp\\]", text) or re.match(r"^bx\\s+lr$", text):
            return {"return_address": ins["address"],
                    "instructions_after_call": distance,
                    "all_paths_verified": False}
        if re.match(r"^pop\\s+\\{[^}]*\\bpc\\b[^}]*\\}$", text):
            return {"return_address": ins["address"],
                    "instructions_after_call": distance,
                    "all_paths_verified": False}
        if CALL.match(text) or re.match(r"^b\\s+", text):
            return None
        # Exclude flag tests and stores that READ r0 but do not replace it.
        if (re.match(r"^[a-z][a-z0-9.]*\\s+r0(?:,|\\s|$)", text)
                and not re.match(r"^(?:str|strb|strh|cmp|cmn|tst|teq)\\b", text)):
            return None
    return None


def scan_callsite(caller_addr: str, instructions: list[dict], target: str) -> list[dict]:
    observed = []
    for pos, inst in enumerate(instructions):
        called = CALL.match(inst["text"])
        if not called or normalize_addr(called.group(1)) != target:
            continue
        next_ins = instructions[pos + 1: pos + 4]
        first = next_ins[0]["text"] if next_ins else ""
        explicit_compare = bool(CHECK_R0.match(first))
        subs_zero = SUBS_R0_ZERO.match(first)
        immediate_test = explicit_compare or subs_zero is not None
        test_kind = ("explicit_flags_test" if explicit_compare else
                     "subs_zero_flags_test" if subs_zero else "not_observed")
        conditional_branch = bool(
            immediate_test and len(next_ins) > 1 and
            CONDITIONAL_BRANCH.match(next_ins[1]["text"])
        )
        copies = []
        for distance, instruction in enumerate(next_ins[:2], 1):
            matched = COPY_R0.match(instruction["text"])
            if matched:
                # A close copy of r0 is evidence that the return is preserved,
                # not proof its value is subsequently checked or propagated.
                copies.append({
                    "address": instruction["address"],
                    "destination_register": matched.group(1),
                    "instructions_after_call": distance,
                })
        passthrough = None
        if not immediate_test and not copies:
            passthrough = normal_fallthrough_return_preserves_r0(
                instructions, pos)
        observed.append({
            "caller": caller_addr, "callee": target, "bl_address": inst["address"],
            "next_three": next_ins,
            "immediate_r0_compare": explicit_compare,
            "immediate_r0_test": immediate_test,
            "r0_test_kind": test_kind,
            "conditional_branch_after_compare": conditional_branch,
            "sub_zero_destination": subs_zero.group(1) if subs_zero else None,
            "r0_copies": copies,
            "r0_preserved_to_normal_return": passthrough,
            "observation": (
                "Return status tested by " + test_kind + " and conditional branch"
                if conditional_branch else
                "Return status tested by " + test_kind
                if immediate_test else
                "Returned r0 copied into register; downstream use not yet traced"
                if copies else
                "Return register r0 preserved along normal fallthrough to epilogue"
                if passthrough else
                "BL call observed; subsequent r0 use requires further analysis"
            ),
        })
    return observed


def detect_return_register_writes(instructions: list[dict]) -> list[dict]:
    sites = []
    for ins in instructions:
        match = SET_R0.match(ins["text"])
        if not match:
            continue
        operand = int(match.group(2), 0)
        if match.group(1).lower() == "mvn":
            operand = (~operand) & 0xffffffff
            if operand >= 2**31:
                operand -= 2**32
        sites.append({"address": ins["address"], "instruction": ins["text"],
                      "value_int32": operand if operand < 2**31 else operand-2**32})
    return sites


def summarize(ghidra: dict, index_check: dict, framing: dict | None = None) -> dict:
    if ghidra.get("schema") != "mpfree-v11-backend-selector-and-boot-recovery-static":
        raise ValueError("Unexpected Ghidra schema")
    if ghidra.get("role") != "libaz01-prog":
        raise ValueError("Wrong Ghidra input role")
    if ghidra.get("source_sha256_prechecked_by_launcher") != EXPECTED_LIBRARY_SHA:
        raise ValueError("Unexpected source library SHA-256")
    functions = ghidra.get("analyzed_functions", {})
    if not isinstance(functions, dict):
        raise ValueError("Ghidra functions must be a mapping")
    analyzed = []
    for target, role in TARGETS.items():
        fn = functions.get(target, {})
        body = str(fn.get("pseudocode", "")).lstrip()
        ins = instructions_for(fn)
        callsites = []
        for caller, caller_fn in functions.items():
            src_ins = instructions_for(caller_fn)
            if src_ins:
                callsites += scan_callsite(caller, src_ins, target)
        analyzed.append({
            "function": target, "role": role,
            "ghidra_void_signature": body.startswith("void "),
            "machine_instructions_captured": len(ins),
            "machine_listing_truncated": bool(fn.get("instruction_capture_limited", False)),
            "r0_constant_writes": detect_return_register_writes(ins),
            "observed_callsites": callsites,
            "callers_immediately_comparing_r0": sorted({
                c["caller"] for c in callsites if c["immediate_r0_compare"]
            }),
            "callers_immediately_testing_r0": sorted({
                c["caller"] for c in callsites if c["immediate_r0_test"]
            }),
            "callers_copying_r0": sorted({
                c["caller"] for c in callsites if c["r0_copies"]
            }),
            "callers_preserving_r0_to_normal_return": sorted({
                c["caller"] for c in callsites
                if c["r0_preserved_to_normal_return"]
            }),
            "instruction_evidence_sha256": hashlib.sha256(
                json.dumps(fn.get("machine_instruction_evidence", []), sort_keys=True).encode()
            ).hexdigest() if ins else None,
            "caution": "A register write is not proof of its reachability at a particular return.",
        })
    return {
        "schema": "mpfree-private-arm-return-contracts-v1",
        "input_source_library_sha256": EXPECTED_LIBRARY_SHA,
        "archive_integrity": index_check,
        "original_vendor_code_executed": False,
        "functions": analyzed,
        "response_predicates_from_framing_analyzer": (
            framing.get("response_validation_checks") if isinstance(framing, dict) else None
        ),
        "interpretation_limits": [
            "ARM AAPCS normally returns integer status in r0; actual callers must be examined.",
            "Ghidra void inference is contradicted where a caller BL is followed by CMP r0 or SUBS rx,r0,#0 and a conditional branch.",
            "SUBS rx,r0,#0 tests condition flags AND copies the return value; do not mistake it for unused r0.",
            "CPY/MOV rx,r0 demonstrates return-value capture, not yet a verified downstream status check.",
            "Fallthrough-preserved r0 to a normal epilogue verifies one path only; exception/branch paths remain distinct.",
            "Only captured instruction evidence is assessed; missing callsites are not proof of absent callers.",
            "A written r0 value is only a candidate return until its control flow to an epilogue is checked.",
            "No actual AKAI updater code was executed and no device was accessed.",
        ],
    }


def to_markdown(report: dict) -> str:
    lines = [
        "# MPFree private ARM return-contract analysis", "",
        "This report comes from the owner's SHA-verified integrated Ghidra ZIP.",
        "It contains small callsite snippets but no proprietary pseudocode.", "",
        "## Return-register and caller evidence",
    ]
    for item in report["functions"]:
        lines.extend(["", "### " + item["function"] + " — " + item["role"],
                      "- Ghidra inferred void return: " + str(item["ghidra_void_signature"]),
                      "- Captured ARM instructions: " + str(item["machine_instructions_captured"]),
                      "- Callers immediately testing r0 (CMP/TST/CMN/TEQ or SUBS-zero): " +
                      (", ".join(item["callers_immediately_testing_r0"]) or "none in captured evidence"),
                      "- Callers copying r0: " +
                      (", ".join(item["callers_copying_r0"]) or "none in captured evidence"),
                      "- Callers preserving r0 to a normal return: " +
                      (", ".join(item["callers_preserving_r0_to_normal_return"])
                       or "none in captured evidence")])
        for site in item["r0_constant_writes"]:
            lines.append(f"- r0 write {site['address']}: {site['instruction']} (int32 {site['value_int32']})")
        for call in item["observed_callsites"]:
            lines.append("- BL caller " + call["caller"] + " @" + call["bl_address"] +
                         ": " + call["observation"])
    lines.extend(["", "## Evidence boundaries"])
    lines.extend("- " + item for item in report["interpretation_limits"])
    return "\n".join(lines) + "\n"


def generate(input_zip: Path, output_zip: Path) -> dict:
    if input_zip.is_symlink() or not input_zip.is_file() or not stat.S_ISREG(input_zip.lstat().st_mode):
        raise ValueError("Input ZIP must be a regular file, not symlink/device")
    if output_zip.exists() or output_zip.is_symlink():
        raise ValueError("Refusing to overwrite output path")
    with zipfile.ZipFile(input_zip) as source:
        integrity = validate_index(source)
        ghidra = json.loads(source.read(LIB))
        framing = json.loads(source.read(FRAMING)) if FRAMING in source.namelist() else None
    report = summarize(ghidra, integrity, framing)
    objects = {
        "arm-return-contracts.json": (json.dumps(report, indent=2) + "\n").encode(),
        "arm-return-contracts.md": to_markdown(report).encode(),
    }
    index = {name: {"bytes": len(blob), "sha256": hashlib.sha256(blob).hexdigest()}
             for name, blob in objects.items()}
    objects["INDEX.json"] = (json.dumps(index, indent=2) + "\n").encode()
    output_zip.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output_zip, "x", compression=zipfile.ZIP_DEFLATED) as result:
        for name, blob in objects.items():
            result.writestr(name, blob)
    return {"out": str(output_zip), "source_index_count": integrity["indexed_members_verified"],
            "functions": len(report["functions"]),
            "immediate_r0_compare_callsites": sum(
                len([c for c in fn["observed_callsites"] if c["immediate_r0_compare"]])
                for fn in report["functions"]),
            "immediate_r0_test_callsites": sum(
                len([c for c in fn["observed_callsites"] if c["immediate_r0_test"]])
                for fn in report["functions"]),
            "r0_copy_callsites": sum(
                len([c for c in fn["observed_callsites"] if c["r0_copies"]])
                for fn in report["functions"]),
            "normal_fallthrough_r0_passthrough_callsites": sum(
                len([c for c in fn["observed_callsites"]
                     if c["r0_preserved_to_normal_return"]])
                for fn in report["functions"])}


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--input-zip", required=True, type=Path)
    ap.add_argument("--output-zip", required=True, type=Path)
    args = ap.parse_args()
    print(json.dumps(generate(args.input_zip, args.output_zip), indent=2))


if __name__ == "__main__":
    main()
