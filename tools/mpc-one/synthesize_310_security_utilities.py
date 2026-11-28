#!/usr/bin/env python3
"""Read-only static analysis of two new MPC firmware 3.10 security utilities.

Input is SHA-pinned private ELF-disassembly metadata already extracted by
the integrated owner lab. Never runs either executable or accesses /dev,
securityfs, USB, firmware storage, or hardware.
"""
from __future__ import annotations

import hashlib
import json
import re

SCHEMA = "mpfree-310-security-utility-static-v1"
SHA = {
    "/usr/bin/az01-bootloader-sig":
        "b7a82030c85ea90e31533773538abfc537d46982bdeaead6963ec9e5ea9a5861",
    "/usr/sbin/az0x-loadpin-trustpoints":
        "fd59c2946b2f8e4bc1c143d5270881aa2168ad2851c4726748642d2f4e040239",
}
UAPI = "https://codebrowser.dev/linux/linux/include/uapi/linux/loadpin.h.html"
LOADPIN_SET_VERITY = 0x40044c00


def calls(lines: list[str], symbol: str) -> list[dict]:
    matches = []
    escaped = re.escape(symbol)
    pat = re.compile(r"^\s*([0-9a-f]+):\s+bl\s+[0-9a-f]+\s+<" + escaped + r">$")
    for i, line in enumerate(lines):
        m = pat.search(line)
        if m:
            matches.append({"offset": "0x" + m.group(1), "line_index": i})
    return matches


def inspect_310(compare_report: dict) -> dict:
    sources = {}
    for path, expected in SHA.items():
        group = compare_report.get("elfs", {}).get(path, {})
        item = group.get("firmware310", {})
        if not item.get("present") or item.get("sha256") != expected:
            raise ValueError("3.10 utility hash/presence mismatch: " + path)
        raw = item.get("small_elf_complete_text", {})
        if raw.get("disassembler_returncode") != 0:
            raise ValueError("Incomplete static disassembly: " + path)
        lines = raw.get("lines")
        if not isinstance(lines, list) or len(lines) < 100:
            raise ValueError("Missing static disassembly: " + path)
        sources[path] = lines
    sig = sources["/usr/bin/az01-bootloader-sig"]
    loadpin = sources["/usr/sbin/az0x-loadpin-trustpoints"]
    names = ("SHA256_Init@plt", "SHA256_Update@plt", "SHA256_Final@plt",
             "RC4_set_key@plt", "RC4@plt", "memcmp@plt")
    sig_calls = {n: calls(sig,n) for n in names}
    rc4_len512 = any(
        re.search(r"\bmov\s+r1,\s*#512\b", previous)
        for call in sig_calls["RC4@plt"]
        for previous in sig[max(0,call["line_index"]-8):call["line_index"]])
    digest_cmp32 = any(
        re.search(r"\bmov\s+r2,\s*#32\b", previous)
        for call in sig_calls["memcmp@plt"]
        for previous in sig[max(0,call["line_index"]-7):call["line_index"]])
    pin_ioctls = calls(loadpin, "__ioctl_time64@plt")
    ioctl_match = []
    for c in pin_ioctls:
        preceding = loadpin[max(0,c["line_index"]-9):c["line_index"]]
        lo = any(re.search(r"\bmov\s+r1,\s*#19456\b", l) for l in preceding)
        hi = any(re.search(r"\bmovt\s+r1,\s*#16388\b", l) for l in preceding)
        if lo and hi:
            ioctl_match.append(c["offset"])
    result = {
        "schema": SCHEMA,
        "source_file_sha256": SHA,
        "vendor_binary_executed": False,
        "hardware_or_securityfs_access": False,
        "bootloader_sig": {
            "sha256_function_calls": {
                name: [x["offset"] for x in sig_calls[name]]
                for name in ("SHA256_Init@plt","SHA256_Update@plt","SHA256_Final@plt")},
            "rc4_set_key_calls":
                [x["offset"] for x in sig_calls["RC4_set_key@plt"]],
            "rc4_calls": [x["offset"] for x in sig_calls["RC4@plt"]],
            "rc4_observed_length_512": bool(rc4_len512),
            "memcmp_calls": [x["offset"] for x in sig_calls["memcmp@plt"]],
            "memcmp_observed_length_32": bool(digest_cmp32),
            "normal_boot_path_invocation_proven": False,
            "rsa_signature_check_in_uboot_proven": False,
        },
        "loadpin_trustpoints": {
            "ioctl_calls": [c["offset"] for c in pin_ioctls],
            "matched_loadpin_ioctl_call_addresses": ioctl_match,
            "ioctl_request": "0x40044c00" if ioctl_match else None,
            "ioctl_name": ("LOADPIN_IOC_SET_TRUSTED_VERITY_DIGESTS"
                           if ioctl_match else None),
            "uapi_reference": UAPI,
            "loadpin_kernel_enforcement_proven": False,
            "systemd_service_executed_in_qemu": False,
        },
        "limitations": [
            "This is a pattern match against the SHA-verified 3.10 static ARM disassembly.",
            "The userspace ioctl sets trusted dm-verity digests; it does not prove boot-time image authentication.",
            "RC4 and SHA256 imports do not identify the actual checked partition or a ROM-enforced signature scheme.",
            "A running kernel's LoadPin enforcement state requires a separate, authorized hardware-specific check.",
        ],
    }
    return result


def markdown(report: dict) -> str:
    b = report["bootloader_sig"]
    l = report["loadpin_trustpoints"]
    return "\n".join([
        "# MPFree: static 3.10 security-utility behavior",
        "", "Original 3.10 ELF sources SHA pinned; no vendor code executed.",
        "", "## Bootloader-signature utility",
        "- SHA256 calls: " + json.dumps(b["sha256_function_calls"], sort_keys=True),
        "- RC4_set_key: " + str(b["rc4_set_key_calls"]),
        "- RC4: " + str(b["rc4_calls"]),
        "- RC4 length 512: " + str(b["rc4_observed_length_512"]),
        "- memcmp length 32: " + str(b["memcmp_observed_length_32"]),
        "- Invocation in normal boot not established.",
        "", "## LoadPin verity trustpoint utility",
        "- ioctl callsites: " + str(l["ioctl_calls"]),
        "- Verified ioctl: " + str(l["ioctl_request"]) + " / " + str(l["ioctl_name"]),
        "- Linux UAPI reference: " + l["uapi_reference"],
        "- Kernel enforcement and actual service execution unproven.",
        "", "## Interpretation boundaries",
        *("- " + x for x in report["limitations"]),
        "",
    ]) + "\n"
