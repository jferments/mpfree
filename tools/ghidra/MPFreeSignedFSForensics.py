# @runtime PyGhidra
# @category MPFree
"""Static forensic export of a single ARM signed-filesystem ELF in Ghidra.

Run only with a fresh private headless Ghidra project. Does not execute
firmware and does not contact hardware. Output is proprietary-derived and
MUST remain in a private local laboratory, not committed to GitHub.
"""
import hashlib
import json
import re
from datetime import datetime, timezone
from pathlib import Path

from ghidra.app.decompiler import DecompInterface

argv = list(getScriptArgs())
if len(argv) != 2 or argv[0] not in ("3.10.0", "3.10.1"):
    raise ValueError("Expected <3.10.0|3.10.1> /absolute/private/output.json")
version, output = argv
output = Path(output)
if not output.is_absolute() or not output.parent.is_dir():
    raise ValueError("Report path must be absolute and its parent must already exist")
if output.exists():
    raise ValueError("Refusing to overwrite a prior report: " + str(output))

p = currentProgram
if p is None:
    raise RuntimeError("No imported Ghidra program")
lang = str(p.getLanguageID())
if not lang.startswith("ARM:"):
    raise RuntimeError("Unexpected language " + lang + "; expected ARM ELF")

fm = p.getFunctionManager()
listing = p.getListing()
refs = p.getReferenceManager()
symbols = p.getSymbolTable()

# Capture actual Ghidra analysis, not guesses based solely on raw strings.
interface = DecompInterface()
interface.toggleCCode(True)
interface.toggleSyntaxTree(True)
if not interface.openProgram(p):
    raise RuntimeError("Could not open decompiler")

CUES = re.compile(r"(sign|verify|verity|mount|hash|trust|digest|crypto|rsa|sha|key|"
                  r"secure|loadpin|root|loop|dm-|block|/dev/|/secure|ioctl|exec|system|"
                  r"open|read|write|stat|strerror|abort|error|memcmp|strcmp)", re.I)


def ref_targets(address):
    found = []
    for r in refs.getReferencesFrom(address):
        if r.getReferenceType().isCall():
            to = r.getToAddress()
            func = fm.getFunctionAt(to)
            if func is None:
                func = fm.getFunctionContaining(to)
            symbol = symbols.getPrimarySymbol(to)
            name = str(func.getName()) if func is not None else (str(symbol.getName()) if symbol else str(to))
            found.append({"target": str(to), "name": name})
    return found


def export_function(fn):
    row = {"name": str(fn.getName()), "entry": str(fn.getEntryPoint()),
           "is_thunk": bool(fn.isThunk()), "body_bytes": int(fn.getBody().getNumAddresses()),
           "calls": [], "mnemonics": [], "ops": [], "security_strings": []}
    insns = listing.getInstructions(fn.getBody(), True)
    calls = set()
    while insns.hasNext():
        inst = insns.next()
        mnemonic = str(inst.getMnemonicString()).lower()
        row["mnemonics"].append(mnemonic)
        ops = []
        for idx in range(int(inst.getNumOperands())):
            v = str(inst.getDefaultOperandRepresentation(idx))
            # This is a structural diagnostic; keep full operand types separately
            # in the private raw Ghidra project for later inspection.
            v = re.sub(r"0x[0-9a-fA-F]+", "ADDR", v)
            v = re.sub(r"\b[0-9a-fA-F]{6,}\b", "ADDR", v)
            ops.append(v[:80])
        row["ops"].append(mnemonic + " " + ",".join(ops))
        for call in ref_targets(inst.getAddress()):
            key = (call["name"], call["target"])
            if key not in calls:
                calls.add(key)
                row["calls"].append(call)
    row["mnemonic_sha256"] = hashlib.sha256("\n".join(row["mnemonics"]).encode()).hexdigest()
    row["instructions"] = len(row["mnemonics"])
    try:
        result = interface.decompileFunction(fn, 45, monitor)
        if result is not None and result.decompileCompleted() and result.getDecompiledFunction() is not None:
            c = str(result.getDecompiledFunction().getC())
            row["decompiled_c"] = c[:60000]
            row["decomp_truncated"] = len(c) > 60000
        else:
            row["decompile_error"] = str(result.getErrorMessage()) if result else "No decompiler result"
    except Exception as exc:
        row["decompile_error"] = str(exc)[:500]
    return row


# Collect strings and *actual* function references, so absence of a literal
# alone is not misinterpreted as absence of reachable code.
strings = []
seen = set()
iter_data = listing.getDefinedData(True)
while iter_data.hasNext():
    data = iter_data.next()
    try:
        value = data.getValue()
        if value is None:
            continue
        # Limit to likely character data: do not emit full binary structures.
        type_name = str(data.getDataType().getName()).lower()
        if not any(x in type_name for x in ("string", "unicode", "ascii", "utf")):
            continue
        v = str(value).strip('"')
        if len(v) < 3 or len(v) > 500 or (str(data.getAddress()), v) in seen:
            continue
        seen.add((str(data.getAddress()), v))
        uses = []
        for ref in refs.getReferencesTo(data.getAddress()):
            owner = fm.getFunctionContaining(ref.getFromAddress())
            if owner:
                uses.append({"from": str(ref.getFromAddress()), "function": str(owner.getName()),
                             "function_entry": str(owner.getEntryPoint())})
        strings.append({"address": str(data.getAddress()), "text": v, "references": uses[:50]})
    except Exception:
        continue

funcs = []
fn_iter = fm.getFunctions(True)
while fn_iter.hasNext():
    fn = fn_iter.next()
    funcs.append(export_function(fn))

# A Ghidra program might not identify every embedded string. Do not interpret
# an empty string-reference list as a negative security finding.
report = {
    "schema": "mpfree-signedfs-ghidra-functions-v1",
    "version": version,
    "timestamp_utc": datetime.now(timezone.utc).isoformat(),
    "program": str(p.getName()),
    "language": lang,
    "image_base": str(p.getImageBase()),
    "function_count": len(funcs),
    "string_count": len(strings),
    "functions": funcs,
    "strings": strings,
    "limitations": [
        "Ghidra recovered-function coverage is heuristic; analysis may miss ARM/Thumb functions.",
        "Decompiled C is approximate and not original source.",
        "A similar function signature does not establish equivalent cryptographic behavior.",
        "Static reports do not establish runtime security policy or bootloader state.",
    ],
}
output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print("MPFree signed FS report: " + str(output))
print("Functions: %d; defined strings: %d" % (len(funcs), len(strings)))
interface.dispose()
