# @runtime PyGhidra
# @category MPFree
"""Read-only (except local Ghidra analysis metadata) RK3288 SPL SRAM audit.

For a fresh imported p1-derived image, preScript seeds the candidate ARM-state
entrypoint at 0xff704004; postScript exports call graph, decompilation, and
indicators of possible Rockchip back-to-BootROM return. This does NOT issue
any hardware commands. A constant scan is only a lead, never proof of control flow.
"""
import hashlib
import json
import os
import re
from datetime import datetime
from pathlib import Path

from java.math import BigInteger
from jpype import JArray, JByte
from ghidra.app.cmd.disassemble import DisassembleCommand
from ghidra.app.decompiler import DecompInterface
from ghidra.program.model.address import AddressSet
from ghidra.program.model.lang import RegisterValue

BASE = 0xFF704000
SIZE = 32768
ENTRY = BASE + 4
MAGIC = b"RK32"
RETURN_TO_ROM_WORD = 0xEF08A53C  # U-Boot's back-to-BROM magic; use as heuristic only.
MAX_FUNCTIONS = 120
MAX_DECOMPILE = 12
OUT = Path(os.environ.get("MPFREE_REPORT_DIR") or
           str(Path.home() / "mpfree-work"))

args = list(getScriptArgs())
phase = args[0] if args else "post"
if phase not in ("pre", "post"):
    raise RuntimeError("MPFree SPL audit phase must be pre or post")

program = currentProgram
if program is None:
    raise RuntimeError("No open Ghidra program")
if str(program.getLanguageID()) != "ARM:LE:32:v7":
    raise RuntimeError("Unexpected architecture: %s" % program.getLanguageID())

memory = program.getMemory()
factory = program.getAddressFactory().getDefaultAddressSpace()
at = lambda n: factory.getAddress(n)
block = memory.getBlock(at(BASE))
if block is None or block.getStart().getOffset() != BASE or block.getSize() != SIZE:
    raise RuntimeError("Expected one 32KiB raw RK32 stage mapped at 0xff704000")

# Validate bytes in Ghidra against the original locally extracted stage.
buffer = JArray(JByte)(4096)
digest = hashlib.sha256()
data = bytearray()
for offset in range(0, SIZE, len(buffer)):
    n = min(len(buffer), SIZE - offset)
    got = int(memory.getBytes(at(BASE + offset), buffer, 0, n))
    if got != n:
        raise RuntimeError("Short memory read at %08x" % (BASE + offset))
    chunk = bytes(int(buffer[i]) & 255 for i in range(n))
    digest.update(chunk)
    data.extend(chunk)
if data[:4] != MAGIC:
    raise RuntimeError("RK32 marker missing from imported SRAM-stage bytes")

if phase == "pre":
    print("MPFree SPL: stage SHA256", digest.hexdigest())
    print("MPFree SPL: mapping correct; seeding candidate entry ARM mode",
          hex(ENTRY))
    reg = program.getProgramContext().getRegister("TMode")
    if reg is None:
        raise RuntimeError("Ghidra ARM TMode register not found")
    # RK3288 ROM initial target state is expected ARM; the image must still
    # be independently checked. Never assume a decoded instruction is correct.
    cmd = DisassembleCommand(AddressSet(at(ENTRY), at(ENTRY)), None, True)
    cmd.setInitialContext(RegisterValue(reg, BigInteger.ZERO))
    result = bool(cmd.applyTo(program, monitor))
    print("MPFree SPL: entry seed result", result)
    if program.getListing().getInstructionAt(at(ENTRY)) is None:
        print("MPFree SPL WARNING: ARM-state entry disassembly not recognized")
    else:
        fm = program.getFunctionManager()
        if fm.getFunctionAt(at(ENTRY)) is None:
            if fm.getFunctionContaining(at(ENTRY)) is None:
                try:
                    createFunction(at(ENTRY), "mpfree_candidate_spl_entry")
                except Exception as exc:
                    print("MPFree SPL: entry function creation unresolved:", exc)
    print("MPFree SPL: automatic analysis may now run")
else:
    fm = program.getFunctionManager()
    listing = program.getListing()
    rm = program.getReferenceManager()
    matches = [
        i for i in range(0, len(data) - 3)
        if int.from_bytes(data[i:i+4], "little") == RETURN_TO_ROM_WORD
    ]
    report = {
        "program": str(program.getName()),
        "architecture": str(program.getLanguageID()),
        "base": "0xff704000",
        "candidate_entry": "0xff704004",
        "stage_sha256": digest.hexdigest(),
        "size_bytes": SIZE,
        "rk32_header": True,
        "assumed_initial_mode": "ARM32; to be confirmed",
        "return_to_brom_magic": "0xef08a53c",
        "return_to_brom_literal_hits": [
            "0x%08x" % (BASE + o) for o in matches[:100]
        ],
        "return_to_brom_literal_hit_count": len(matches),
        "important_caveats": [
            "A magic word may appear as data or be synthesized without a literal.",
            "The presence or absence of this constant is not proof of back-to-BROM.",
            "The function inventory is heuristic Ghidra auto-analysis, not verified CPU execution.",
            "No MPC, eMMC, USB, OTP, firmware or board state was accessed.",
            "The decompiler output may contain vendor proprietary firmware; keep the report local/private."
        ],
        "entry_instructions": [],
        "functions": [],
    }

    instruction = listing.getInstructionAt(at(ENTRY))
    if instruction:
        # Read only the first decoded instruction sequence, not raw payload.
        cur = instruction
        for _ in range(16):
            if cur is None:
                break
            if cur.getAddress().getOffset() >= BASE + SIZE:
                break
            report["entry_instructions"].append({
                "address": str(cur.getAddress()),
                "mnemonic": str(cur.getMnemonicString()),
                "operands": str(cur),
            })
            cur = listing.getInstructionAfter(cur.getAddress())

    dc = DecompInterface()
    dc.toggleCCode(True)
    dc.setSimplificationStyle("decompile")
    opened = dc.openProgram(program)
    try:
        functions = []
        it = fm.getFunctions(True)
        while it.hasNext() and len(functions) < MAX_FUNCTIONS:
            fn = it.next()
            va = int(fn.getEntryPoint().getOffset())
            if BASE <= va < BASE + SIZE:
                functions.append(fn)

        # Prioritize the entrypoint and immediate proximity, then scan all.
        priority = sorted(functions, key=lambda f: (
            0 if f.getEntryPoint().getOffset() == ENTRY else 1,
            abs(f.getEntryPoint().getOffset() - ENTRY)
        ))
        allowed = set(int(f.getEntryPoint().getOffset())
                      for f in priority[:MAX_DECOMPILE])
        for fn in functions:
            va = int(fn.getEntryPoint().getOffset())
            row = {
                "entry": "0x%08x" % va,
                "ghidra_name": str(fn.getName()),
                "size_bytes": int(fn.getBody().getNumAddresses()),
            }
            refs = rm.getReferencesTo(fn.getEntryPoint())
            inbound = []
            while refs.hasNext() and len(inbound) < 40:
                ref = refs.next()
                inbound.append({"from": str(ref.getFromAddress()),
                                "type": str(ref.getReferenceType())})
            row["inbound"] = inbound

            calls = []
            ins = listing.getInstructions(fn.getBody(), True)
            count = 0
            while ins.hasNext() and count < 10000:
                inst = ins.next()
                count += 1
                if inst.getFlowType().isCall():
                    calls.append({"from": str(inst.getAddress()),
                                  "to": [str(a) for a in inst.getFlows()]})
            row["calls"] = calls[:100]

            if opened and va in allowed:
                try:
                    dec = dc.decompileFunction(fn, 60, monitor)
                    if dec and dec.decompileCompleted() and dec.getDecompiledFunction():
                        code = str(dec.getDecompiledFunction().getC())
                        code = re.sub(r'"[^"\n]{35,}"',
                                      '"[LONG_LITERAL_REDACTED]"', code)
                        row["pseudocode"] = code[:24000]
                        row["truncated"] = len(code) > 24000
                    elif dec:
                        row["error"] = str(dec.getErrorMessage())
                except Exception as ex:
                    row["error"] = str(ex)
            report["functions"].append(row)
    finally:
        dc.dispose()

    OUT.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%dT%H%M%S")
    destination = OUT / ("mpfree-ghidra-p1-sram-" + stamp + ".json")
    destination.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print("MPFree SPL audit REPORT:", destination)
    print("MPFree SPL recognized functions:", len(report["functions"]))
    print("MPFree SPL back-to-ROM magic occurrences:", len(matches))
    print("MPFree SPL: no device, eMMC, ROM or OTP operations performed")
