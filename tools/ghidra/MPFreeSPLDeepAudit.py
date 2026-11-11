# @runtime PyGhidra
# @category MPFree
"""Read-only second-pass for the already analyzed MPC One RK3288 SRAM SPL.

This file does NOT modify Ghidra's project or contact the MPC. It characterizes
the decoded startup path, every discovered function, MMIO/storage-controller
references, and selected deep decompilation. Findings require human review.
"""
import hashlib
import json
import os
import re
from collections import deque
from datetime import datetime
from pathlib import Path

from ghidra.app.decompiler import DecompInterface
from jpype import JArray, JByte

BASE = 0xFF704000
LENGTH = 32768
EXPECTED_SHA = "6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"
CHIP_MARKER = b"RK32"
OUT_DIR = Path(os.environ.get("MPFREE_REPORT_DIR", str(Path.home() / "mpfree-work")))
STARTUP = [BASE + x for x in (0x04, 0x20, 0x60, 0x64, 0x68, 0x94, 0x98,
                               0x9c, 0xa0, 0xb4, 0xb8, 0x118, 0x1e4, 0x1e8,
                               0x208, 0x1208, 0x1350, 0x14a4)]
# SOC register families (addresses must be independently confirmed against TRM).
MMIO_HINTS = {
    0xFF0C0000: "RK3288 SDMMC controller (candidate)",
    0xFF0D0000: "RK3288 SDIO controller (candidate)",
    0xFF0E0000: "RK3288 SDIO controller (candidate)",
    0xFF0F0000: "RK3288 eMMC controller (candidate)",
    0xFF760000: "RK3288 GRF (candidate)",
    0xFF770000: "RK3288 USB PHY SYS-CON (candidate)",
    0xFF730000: "RK3288 CRU (candidate; confirm TRM address)",
    0xFFB40000: "RK3288 eFuse controller (candidate)",
    0xFFB10000: "RK3288 secure eFuse bank (candidate)",
}
BOOTROM_MAGIC = 0xEF08A53C

program = currentProgram
if program is None or str(program.getLanguageID()) != "ARM:LE:32:v7":
    raise RuntimeError("Expected an analyzed ARM:LE:32:v7 SRAM program")

memory = program.getMemory()
space = program.getAddressFactory().getDefaultAddressSpace()
addr = lambda n: space.getAddress(n)
block = memory.getBlock(addr(BASE))
if block is None or block.getStart().getOffset() != BASE or block.getSize() != LENGTH:
    raise RuntimeError("Expected RK32 32768-byte block at ff704000")

buffer = JArray(JByte)(4096)
data = bytearray()
for off in range(0, LENGTH, len(buffer)):
    count = min(len(buffer), LENGTH - off)
    read = int(memory.getBytes(addr(BASE + off), buffer, 0, count))
    if read != count:
        raise RuntimeError("Short SRAM program memory read at 0x%x" % off)
    data.extend(int(buffer[i]) & 0xFF for i in range(count))
sha = hashlib.sha256(data).hexdigest()
if data[:4] != CHIP_MARKER or sha != EXPECTED_SHA:
    raise RuntimeError("Wrong firmware or incorrect program mapping: sha256=%s" % sha)

listing = program.getListing()
fm = program.getFunctionManager()
rm = program.getReferenceManager()

def word(va):
    return int.from_bytes(data[va - BASE:va - BASE + 4], "little")

def describe_at(va):
    if not BASE <= va < BASE + LENGTH:
        return {"address": "0x%08x" % va, "outside_image": True}
    inst = listing.getInstructionContaining(addr(va))
    fn = fm.getFunctionContaining(addr(va))
    refs = rm.getReferencesTo(addr(va))
    ins = {
        "address": "0x%08x" % va,
        "offset": "0x%04x" % (va - BASE),
        "word_le": "0x%08x" % word(va) if va % 4 == 0 and va + 4 <= BASE + LENGTH else None,
        "classified_as_instruction": bool(inst),
        "function": str(fn.getName()) if fn else None,
    }
    if inst:
        ins.update({
            "instruction_start": str(inst.getAddress()),
            "instruction": str(inst),
            "flow": str(inst.getFlowType()),
            "targets": [str(x) for x in inst.getFlows()],
            "fallthrough": str(inst.getFallThrough()) if inst.getFallThrough() else None,
        })
    inbound = []
    while refs.hasNext() and len(inbound) < 35:
        ref = refs.next()
        inbound.append({
            "from": str(ref.getFromAddress()),
            "type": str(ref.getReferenceType()),
        })
    if inbound:
        ins["inbound_refs"] = inbound
    return ins

report = {
    "schema": "mpfree-spl-deep-audit-v1",
    "program": str(program.getName()),
    "base": "0xff704000",
    "size": LENGTH,
    "sha256": sha,
    "status": "offline static observation only",
    "limits": [
        "Ghidra seeds 0xff704004 as ARM code despite zero padding; do not trust its entry pseudocode alone.",
        "Observed ARM and Thumb mode transitions may be misclassified; inspect raw words and control flow.",
        "0xef08a53c absent as a literal does not establish ROM return is absent.",
        "ROM load ACK would not prove code execution.",
        "This report includes proprietary decompilation and must remain private.",
    ],
    "startup_words": [describe_at(BASE + off) for off in range(0, 0x120, 4)],
    "startup_flow_sites": [describe_at(x) for x in STARTUP],
    "mmio_exact_literals": [],
    "internal_address_constants": [],
    "brom_magic_le_offsets": [],
    "function_count_total": 0,
    "function_summary": [],
    "startup_reachable": [],
    "decompilation": {},
}

for off in range(0, LENGTH - 3, 4):
    value = int.from_bytes(data[off:off+4], "little")
    if value == BOOTROM_MAGIC:
        report["brom_magic_le_offsets"].append("0x%08x" % (BASE + off))
    if value in MMIO_HINTS:
        location = BASE + off
        f = fm.getFunctionContaining(addr(location))
        report["mmio_exact_literals"].append({
            "location": "0x%08x" % location,
            "value": "0x%08x" % value,
            "hint": MMIO_HINTS[value],
            "function": str(f.getName()) if f else None,
        })
    if BASE <= value < BASE + LENGTH and value % 2 == 0:
        report["internal_address_constants"].append({
            "location": "0x%08x" % (BASE + off),
            "value": "0x%08x" % value,
        })
report["internal_address_constants"] = report["internal_address_constants"][:256]

all_functions = {}
it = fm.getFunctions(True)
while it.hasNext():
    f = it.next()
    va = int(f.getEntryPoint().getOffset())
    if BASE <= va < BASE + LENGTH:
        all_functions[va] = f

report["function_count_total"] = len(all_functions)
call_graph = {}
for va, f in all_functions.items():
    direct = set()
    indirect = 0
    ins = listing.getInstructions(f.getBody(), True)
    count = 0
    while ins.hasNext() and count < 20000:
        i = ins.next()
        count += 1
        if i.getFlowType().isCall():
            targets = [int(t.getOffset()) for t in i.getFlows()]
            for target in targets:
                if BASE <= target < BASE + LENGTH:
                    containing = fm.getFunctionContaining(addr(target))
                    if containing:
                        direct.add(int(containing.getEntryPoint().getOffset()))
            if not targets:
                indirect += 1
    call_graph[va] = sorted(direct)
    report["function_summary"].append({
        "entry": "0x%08x" % va,
        "name": str(f.getName()),
        "bytes": int(f.getBody().getNumAddresses()),
        "calls": ["0x%08x" % t for t in sorted(direct)],
        "indirect_calls": indirect,
    })

# Syntactic reachability in the already-decoded Ghidra graph. Do not equate it
# to dynamic execution, especially until correct ARM/Thumb states are verified.
queue = deque(va for va in (BASE + 0x04, BASE + 0x60,
                            BASE + 0x1208, BASE + 0x1350) if va in all_functions)
seen = set()
while queue and len(seen) < 500:
    va = queue.popleft()
    if va in seen:
        continue
    seen.add(va)
    queue.extend(t for t in call_graph.get(va, []) if t not in seen)
report["startup_reachable"] = ["0x%08x" % x for x in sorted(seen)]

# Prioritize startup, then likely MMC / SoC / ROM call-heavy routines.
priorities = [BASE + off for off in (0x04, 0x60, 0xb8, 0x118,
                                     0x164, 0x1a8, 0x1e4, 0x1e8,
                                     0x208, 0x1208, 0x1350, 0x14a4,
                                     0x21c0, 0x2a4c)]
# Include functions containing suspicious exact MMIO base constants, if any.
for item in report["mmio_exact_literals"]:
    f = fm.getFunctionContaining(addr(int(item["location"], 16)))
    if f:
        priorities.append(int(f.getEntryPoint().getOffset()))
priorities = list(dict.fromkeys(priorities))

decompiler = DecompInterface()
decompiler.toggleCCode(True)
decompiler.setSimplificationStyle("decompile")
if not decompiler.openProgram(program):
    raise RuntimeError("Decompiler failed to open SRAM program")
try:
    for target in priorities[:25]:
        f = fm.getFunctionContaining(addr(target))
        key = "0x%08x" % target
        if not f:
            report["decompilation"][key] = {"warning": "No recognized function"}
            continue
        try:
            dec = decompiler.decompileFunction(f, 60, monitor)
            if dec and dec.decompileCompleted() and dec.getDecompiledFunction():
                code = str(dec.getDecompiledFunction().getC())
                code = re.sub(r'"[^"\r\n]{35,}"', '"[LARGE_STRING_REDACTED]"', code)
                report["decompilation"][key] = {
                    "function_entry": str(f.getEntryPoint()),
                    "code": code[:32000],
                    "truncated": len(code) > 32000,
                }
            else:
                report["decompilation"][key] = {
                    "error": str(dec.getErrorMessage()) if dec else "no result"
                }
        except Exception as err:
            report["decompilation"][key] = {"exception": str(err)}
finally:
    decompiler.dispose()

args = list(getScriptArgs())
dst = Path(str(args[0])) if args else (OUT_DIR / (
    "mpfree-ghidra-p1-deep-" + datetime.utcnow().strftime("%Y%m%dT%H%M%SZ") + ".json"
))
dst.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
dst.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print("MPFree SPL deep report:", dst)
print("Functions enumerated:", report["function_count_total"])
print("Startup graph function count:", len(seen))
print("Exact MMC/eFuse MMIO literal hints:", len(report["mmio_exact_literals"]))
print("Back-to-BROM magic literal occurrences:", len(report["brom_magic_le_offsets"]))
print("Read-only study complete; no hardware or firmware writes")
