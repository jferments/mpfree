# @runtime PyGhidra
# @category MPFree
"""MPFree V13: targeted, SHA-pinned, READ-ONLY Fastboot pointer/dispatch audit.

Examine owner-saved P1/P3 Ghidra projects with -readOnly -noanalysis.
No execution of vendor firmware, U-Boot, or update tools, no MPC contact.
Output contains proprietary derived evidence; keep it in private ZIP only.
"""
import hashlib
import json
import os
from datetime import datetime, timezone
from pathlib import Path
from ghidra.app.decompiler import DecompInterface
from jpype import JArray, JByte

IMAGES = {
    "p1": (0xff704000, 32768,
           "6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"),
    "p3": (0x00100000, 363649,
           "b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"),
}
CELLS = {
    "p3": {
        "permission_gate_pointer": 0x0012284c,
        "oem_unlock_writer_pointer": 0x00122928,
        "fastboot_dispatch_table_pointer": 0x00122818,
        "unlock_command_text_pointer": 0x00122924,
        "fastboot_download_buffer_pointer": 0x001229f4,
        "fastboot_download_size_pointer": 0x001229f8,
        "fastboot_download_done_callback": 0x00122a0c,
        "fastboot_boot_state_pointer": 0x00122974,
        "fastboot_boot_state_value": 0x00122978,
        "fastboot_boot_response_pointer": 0x0012297c,
        "fastboot_bootm_download_addr_pointer": 0x001226fc,
        "fastboot_bootm_command_format_pointer": 0x00122700,
        "fastboot_bootm_initial_pointer": 0x001226f8,
        "bootm_global_context": 0x001017dc,
        "saved_boot_targets_literal_candidate": 0x0013363f,
    },
    "p1": {
        "spl_image_selector": 0xff70530c,
        "spl_boot_media_table_candidate": 0xff705350,
        "spl_pmu_reset_literal": 0xff705290,
    },
}
TARGETS = {
    "p3": {
        "oem_unlock_and_permission": [0x00122834, 0x00122854, 0x0012288e,
                                      0x00122892, 0x0012289a, 0x0012289e,
                                      0x00122928, 0x0014d690],
        "download_boot_state": [0x00122780, 0x00122964, 0x00122980,
                                0x00122a10, 0x0012247c, 0x001226b4,
                                0x00101744, 0x001017fc, 0x0010a6a4],
        "flash_erase_oem": [0x00122bf8, 0x00122c5c, 0x00122cf8,
                            0x00122734],
        "boot_selection": [0x00100fe4, 0x001011e0, 0x00106000,
                           0x001095f8, 0x00110484, 0x0010e000,
                           0x0010e390, 0x0010f388],
        "pmu_button_and_recovery": [0x00100edc, 0x0010107e, 0x001010a0,
                                    0x001010aa, 0x0010114c,
                                    0x00112b20, 0x0011622a],
    },
    "p1": {
        "rom_and_handoff": [0xff704004, 0xff704060, 0xff705208,
                            0xff705310, 0xff705350, 0xff7054a4,
                            0xff7056a8],
        "boot_media_and_recovery": [0xff705316, 0xff708b74, 0xff708cdc,
                                    0xff708f98, 0xff707f48],
    }
}
FOCUS_CONSTANTS = {
    "vendor_permission_flag_candidate": 0x0014d690,
    "pmu_reboot_address": 0xff730094,
    "pmu_bootcount_address_candidate": 0xff7300a0,
    "pmu_bootcount_magic_candidate": 0xb0010000,
    "rk3288_rom_download_magic": 0xef08a53c,
    "fastboot_rx_address_candidate": 0x00800800,
    "vendor_kernel_ram_address": 0x02000000,
    "vendor_dtb_ram_address": 0x01f00000,
    "vendor_ramdisk_address": 0x04000000,
    "fastboot_advertised_maximum": 0x70000000,
}
args = list(getScriptArgs())
if len(args) != 2 or str(args[0]) not in IMAGES:
    raise RuntimeError("Expected role p1|p3 and absolute JSON output path")
role = str(args[0])
output_path = Path(str(args[1]))
if not output_path.is_absolute(): raise RuntimeError("Absolute output path required")
base, length, source_sha = IMAGES[role]
p = currentProgram
if p is None or str(p.getLanguageID()) != "ARM:LE:32:v7":
    raise RuntimeError("Expected ARM:LE:32:v7 program")
memory = p.getMemory()
block = memory.getBlock(p.getAddressFactory().getDefaultAddressSpace().getAddress(base))
if block is None or block.getStart().getOffset() != base or block.getSize() != length:
    raise RuntimeError("Unexpected owner firmware memory block")
space = p.getAddressFactory().getDefaultAddressSpace()
listing = p.getListing()
refs = p.getReferenceManager()
fm = p.getFunctionManager()
def A(n): return space.getAddress(int(n))
def H(n): return "0x%08x" % int(n)
buf = JArray(JByte)(32768)
raw = bytearray()
for offset in range(0, length, 32768):
    n = min(32768, length - offset)
    if int(memory.getBytes(A(base + offset), buf, 0, n)) != n:
        raise RuntimeError("Unexpected memory truncation")
    raw.extend(int(b) & 255 for b in buf[:n])
raw = bytes(raw)
if hashlib.sha256(raw).hexdigest() != source_sha:
    raise RuntimeError("Saved program image differs from pinned owner source hash")
def read_u32(address):
    if base <= address <= base + length - 4:
        return int.from_bytes(raw[address - base: address - base + 4], "little")
    return None
def sanitized_text(address, max_bytes=100):
    if address is None or not base <= address < base + length: return None
    contents = raw[address - base:address - base + max_bytes]
    contents = contents.split(b"\x00", 1)[0]
    if len(contents) < 3 or any(b < 32 or b >= 127 for b in contents):
        return None
    s = contents.decode("ascii")
    if "unlock" in s.lower() or "magic" in s.lower():
        return "<REDACTED_VENDOR_AUTH_STRING>"
    if any(word in s.lower() for word in ("password", "serial", "secret", "token")):
        return "<REDACTED_SENSITIVE_STRING>"
    return s[:76]
def xrefs(address, cap=200):
    rows = []
    try:
        for ref in refs.getReferencesTo(A(address)):
            loc = int(ref.getFromAddress().getOffset())
            fn = fm.getFunctionContaining(A(loc))
            rows.append({
                "from": H(loc), "type": str(ref.getReferenceType()),
                "function": H(fn.getEntryPoint().getOffset()) if fn else None,
            })
            if len(rows) >= cap: break
    except Exception as exc:
        rows.append({"error": str(exc)[:200]})
    return rows
def instructions(address, before=12, after=100, max_lines=95):
    if not base <= address < base + length: return []
    cur = listing.getInstructionContaining(A(max(base, address - before)))
    if cur is None: cur = listing.getInstructionAfter(A(max(base, address - before)))
    rows = []
    while cur is not None and len(rows) < max_lines:
        at = int(cur.getAddress().getOffset())
        if at > address + after or at >= base + length: break
        rows.append({"address": H(at), "instruction": str(cur)[:220],
                     "flows": [str(t) for t in cur.getFlows()][:7]})
        cur = cur.getNext()
    return rows
report = {
    "schema": "mpfree-v13-fastboot-pointer-dispatch",
    "generated_utc": datetime.now(timezone.utc).isoformat(),
    "role": role, "source_sha256": source_sha, "source_verified": True,
    "hardware_contact": False, "vendor_code_executed": False,
    "pointer_cells": {}, "alias_groups": [], "dispatch_table": {},
    "focused_functions": {}, "function_details": {}, "constant_occurrences": {},
    "limitations": [
        "Static pointers in a saved image are not proof of live initialization.",
        "A synthetic test cannot verify vendor Fastboot behavior.",
        "ARM/Thumb entrypoints and Ghidra decompilation can be imperfect.",
        "No ROM, USB, eMMC or vendor executable was touched.",
    ],
}
for name, cell in CELLS[role].items():
    value = read_u32(cell)
    report["pointer_cells"][name] = {
        "cell": H(cell), "value": H(value) if value is not None else None,
        "points_within_pinned_payload": bool(value is not None and base <= value < base + length),
        "target_xrefs": xrefs(value, 150) if value is not None else [],
        "static_string_target": sanitized_text(value),
    }
groups = {}
for key, cell in report["pointer_cells"].items():
    if cell["value"]:
        groups.setdefault(cell["value"], []).append(key)
report["alias_groups"] = [{"address": target, "names": keys}
                          for target, keys in groups.items() if len(keys) > 1]
if role == "p3":
    expected = ("permission_gate_pointer", "oem_unlock_writer_pointer")
    a, b = [report["pointer_cells"][name]["value"] for name in expected]
    report["oem_permission_alias"] = {
        "gate_pointer": a, "unlock_writer_pointer": b,
        "same_nonzero_address": bool(a and b and a == b and a != "0x00000000"),
        "note": "Proves static pointer alias only, not physical Fastboot authorization",
    }
    ptr = read_u32(CELLS[role]["fastboot_dispatch_table_pointer"])
    table = {"table_start": H(ptr) if ptr is not None else None, "entries": []}
    if ptr is not None and base <= ptr and ptr + 8 * 8 <= base + length:
        for i in range(8):
            command_ptr, handler_ptr = read_u32(ptr + i * 8), read_u32(ptr + i * 8 + 4)
            table["entries"].append({
                "index": i,
                "command_pointer": H(command_ptr),
                "handler_pointer": H(handler_ptr),
                "command": sanitized_text(command_ptr),
                "handler_function": str(fm.getFunctionAt(A(handler_ptr)).getName())
                    if fm.getFunctionAt(A(handler_ptr)) else None,
                "handler_xrefs": xrefs(handler_ptr, 50),
            })
    else:
        table["note"] = "Dispatch table pointer not inside the pinned raw payload; needs relocation analysis"
    report["dispatch_table"] = table
for name, value in FOCUS_CONSTANTS.items():
    needle = int(value).to_bytes(4, "little")
    sites = []
    pos = 0
    while len(sites) < 120:
        at = raw.find(needle, pos)
        if at < 0: break
        sites.append(H(base + at))
        pos = at + 1
    report["constant_occurrences"][name] = {
        "value": H(value), "locations": sites,
        "total_captured": len(sites),
        "ghidra_xrefs_to_value": xrefs(value, 80),
        "limitation": "Literal occurrences alone do not establish behavior.",
    }
selected = {}
for group, targets in TARGETS[role].items():
    entries = {}
    for address in targets:
        if not base <= address < base + length:
            continue
        fn = fm.getFunctionContaining(A(address))
        entries[H(address)] = {
            "containing_function": H(fn.getEntryPoint().getOffset()) if fn else None,
            "incoming": xrefs(address, 80),
            "disassembly": instructions(address),
        }
        if fn:
            selected[int(fn.getEntryPoint().getOffset())] = fn
    report["focused_functions"][group] = entries
if role == "p3":
    for address in (0x0014d690, 0x00122834, 0x00122854, 0x00122980, 0x001226b4):
        for ref in xrefs(address, 120):
            if ref.get("function"):
                entry = int(ref["function"], 16)
                fn = fm.getFunctionAt(A(entry))
                if fn: selected[entry] = fn
for key in ("permission_gate_pointer", "oem_unlock_writer_pointer"):
    if key not in report["pointer_cells"]: continue
    ptr = read_u32(CELLS[role][key])
    if ptr is None: continue
    for ref in xrefs(ptr, 150):
        if ref.get("function"):
            entry = int(ref["function"], 16)
            fn = fm.getFunctionAt(A(entry))
            if fn: selected[entry] = fn
if role == "p3":
    for entry in report["dispatch_table"].get("entries", []):
        ptr = int(entry["handler_pointer"], 16)
        fn = fm.getFunctionAt(A(ptr))
        if fn: selected[ptr] = fn
dec = DecompInterface()
dec.toggleCCode(True)
dec.setSimplificationStyle("decompile")
if not dec.openProgram(p): raise RuntimeError("Ghidra decompiler failed to open verified owner project")
try:
    for address, fn in list(selected.items())[:110]:
        detail = {"entry": H(address), "name": str(fn.getName()), "incoming": xrefs(address, 90), "calls": []}
        it = listing.getInstructions(fn.getBody(), True)
        while it.hasNext() and len(detail["calls"]) < 140:
            ins = it.next()
            if ins.getFlowType().isCall():
                detail["calls"].append({"at": str(ins.getAddress()),
                                        "targets": [str(t) for t in ins.getFlows()][:8]})
        try:
            result = dec.decompileFunction(fn, 35, monitor)
            if result and result.decompileCompleted() and result.getDecompiledFunction():
                source = str(result.getDecompiledFunction().getC())
                import re
                source = re.sub(r"inmusic-unlock-magic-[0-9A-Fa-f]+",
                                "inmusic-unlock-magic-[REDACTED]", source)
                detail["pseudocode"] = source[:16000]
                detail["truncated"] = len(source) > 16000
            else:
                detail["decompile_error"] = str(result.getErrorMessage())[:300]
        except Exception as exc:
            detail["decompile_error"] = str(exc)[:300]
        report["function_details"][H(address)] = detail
finally:
    dec.dispose()
report["statistics"] = {
    "pointer_cells": len(report["pointer_cells"]),
    "alias_groups": len(report["alias_groups"]),
    "dispatch_entries": len(report["dispatch_table"].get("entries", [])),
    "selected_functions": len(selected),
    "reported_functions": len(report["function_details"]),
}
old_umask = os.umask(0o077)
try:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf8")
finally:
    os.umask(old_umask)
print("V13 static pointer/boot graph", role, "owner image verified:",
      source_sha, "functions", len(report["function_details"]),
      "private report", str(output_path))
