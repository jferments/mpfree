# @runtime PyGhidra
# @category MPFree
# Read-only follow-up on the correctly based, already-analyzed p3 U-Boot.
# Launch with: snap run ghidra.ghidra -H ... -postScript MPFreePythonFollowup.py
# No connection to MPC, no eMMC write, no project modifications.
import hashlib
import json
import os
import re
from datetime import datetime
from pathlib import Path
from ghidra.app.decompiler import DecompInterface
from jpype import JArray, JByte

BASE = 0x00100000
SIZE = 363649
EXPECTED_SHA = "b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
TARGETS = {
    "fastboot_boot_done": 0x001226B4,
    "fastboot_continue_done": 0x001226B0,
    "fastboot_permission": 0x00122834,
    "fastboot_download": 0x00122980,
    "fastboot_receive": 0x00122A10,
    "fastboot_boot": 0x00122964,
    "fastboot_dispatch": 0x00122780,
    "bootm": 0x00101744,
    "bootm_image_loading": 0x0010A6A4,
    "image_format": 0x0010E390,
    "uimage_header_crc": 0x0010E000,
    "uimage_data_crc": 0x0010E02A,
    "fdt_header_check": 0x00127A1C,
}
GLOBALS = {
    "permission": 0x0014D690,
    "download_received": 0x0014D680,
    "download_total": 0x0014D684,
    "continue_exit": 0x0014D658,
    "download_base": 0x00143E34,
}

program = currentProgram
if program is None:
    raise RuntimeError("Open an analyzed MPC p3 program")
if str(program.getLanguageID()) != "ARM:LE:32:v7":
    raise RuntimeError("Expected ARM:LE:32:v7")

mem = program.getMemory()
space = program.getAddressFactory().getDefaultAddressSpace()
addr = lambda value: space.getAddress(int(value))
block = mem.getBlock(addr(BASE))
if block is None or block.getStart().getOffset() != BASE or block.getSize() != SIZE:
    raise RuntimeError("Not the correctly based 363649-byte p3 raw U-Boot payload")

sha = hashlib.sha256()
buf = JArray(JByte)(32768)
for offset in range(0, SIZE, 32768):
    n = min(32768, SIZE - offset)
    actual = int(mem.getBytes(addr(BASE + offset), buf, 0, n))
    if actual != n:
        raise RuntimeError("Failed reading firmware at +%x" % offset)
    sha.update(bytes(int(byte) & 255 for byte in buf[:n]))
if sha.hexdigest() != EXPECTED_SHA:
    raise RuntimeError("Firmware SHA256 mismatch; wrong program")

def u32(va):
    return sum((mem.getByte(addr(va + j)) & 255) << (8 * j) for j in range(4))

if u32(BASE + 0x2F2B0) != 0x0013F08B or u32(BASE + 0x2F2B4) != 0x00122CF9:
    raise RuntimeError("Fastboot table anchor mismatch")

functions = program.getFunctionManager()
listing = program.getListing()
references = program.getReferenceManager()
decompiler = DecompInterface()
decompiler.toggleCCode(True)
decompiler.setSimplificationStyle("decompile")
if not decompiler.openProgram(program):
    raise RuntimeError("Cannot open Ghidra decompiler")

result = {
    "program": str(program.getName()),
    "base": "0x%08x" % BASE,
    "sha256": sha.hexdigest(),
    "functions": {},
    "globals": {},
    "limitations": [
        "Static analysis; does not demonstrate live MaskROM recovery.",
        "On-disk values are not necessarily initialized runtime values.",
        "RAM safety and complete bootm handoff remain unverified."
    ],
}

try:
    for label, va in TARGETS.items():
        fn = functions.getFunctionAt(addr(va))
        entry = {"address": "0x%08x" % va, "recognized": bool(fn)}
        if fn is None:
            parent = functions.getFunctionContaining(addr(va))
            entry["containing_function"] = (
                str(parent.getEntryPoint()) if parent else None
            )
        else:
            entry["name"] = str(fn.getName())
            incoming = []
            for ref in references.getReferencesTo(addr(va)):
                incoming.append({
                    "at": str(ref.getFromAddress()),
                    "type": str(ref.getReferenceType()),
                })
                if len(incoming) >= 75:
                    break
            entry["inbound_refs"] = incoming
            calls = []
            instructions = listing.getInstructions(fn.getBody(), True)
            count = 0
            while instructions.hasNext() and count < 20000:
                ins = instructions.next()
                count += 1
                if ins.getFlowType().isCall():
                    calls.append({
                        "at": str(ins.getAddress()),
                        "targets": [str(x) for x in ins.getFlows()],
                    })
            entry["calls"] = calls[:150]
            dec = decompiler.decompileFunction(fn, 90, monitor)
            if dec and dec.decompileCompleted() and dec.getDecompiledFunction():
                source = str(dec.getDecompiledFunction().getC())
                source = re.sub(
                    r'"[^"\\n]{30,}"',
                    '"[LONG_STRING_REDACTED]"',
                    source,
                )
                entry["c"] = source[:28000]
                entry["truncated"] = len(source) > 28000
            else:
                entry["error"] = str(dec.getErrorMessage()) if dec else "No result"
        result["functions"][label] = entry

    for label, va in GLOBALS.items():
        xrefs = []
        for ref in references.getReferencesTo(addr(va)):
            xrefs.append({
                "at": str(ref.getFromAddress()),
                "type": str(ref.getReferenceType()),
            })
            if len(xrefs) >= 100:
                break
        result["globals"][label] = {
            "address": "0x%08x" % va,
            "file_word": "0x%08x" % u32(va),
            "refs": xrefs,
        }
finally:
    decompiler.dispose()

script_args = list(getScriptArgs())
output_dir = Path(os.environ.get("MPFREE_REPORT_DIR", "/home/jesse/mpfree-work"))
destination = (
    Path(str(script_args[0])) if script_args else
    output_dir / ("mpfree-ghidra-p3-python-" +
                  datetime.now().strftime("%Y%m%dT%H%M%S") + ".json")
)
destination.parent.mkdir(parents=True, exist_ok=True)
destination.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
print("MPFree PyGhidra 3 REPORT:", destination)
