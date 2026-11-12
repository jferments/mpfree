# @runtime PyGhidra
# @category MPFree
"""V12 read-only vendor U-Boot/SPL forensic pass.

No device access, no firmware execution, no patching of saved programs.
Run with -readOnly -noanalysis on SHA-verified Ghidra projects.
Reports include proprietary decompilation: NEVER commit the output.
"""
import hashlib
import json
import os
import re
from pathlib import Path
from datetime import datetime, timezone
from ghidra.app.decompiler import DecompInterface
from jpype import JArray, JByte

IMAGES = {
    "p1": (0xff704000, 32768,
           "6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"),
    "p3": (0x00100000, 363649,
           "b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"),
}
TARGETS = {
    "p1": {
        "rom_and_spl_start": [0xff704020, 0xff704064, 0xff7040b4, 0xff7041e4,
                              0xff705208, 0xff705274],
        "boot_media_and_sd": [0xff705310, 0xff705350, 0xff705398, 0xff7053c2,
                              0xff7054a4, 0xff705598],
        "rom_return_reset": [0xff705316, 0xff708b74, 0xff708cdc,
                             0xff7041a8, 0xff707f48],
    },
    "p3": {
        "fastboot_authorization": [0x00122834, 0x00122854, 0x0012289e,
                                   0x00122948, 0x00122964, 0x00122980,
                                   0x00122a10, 0x00122bf8, 0x00122c5c,
                                   0x00122cf8, 0x0014d690],
        "fastboot_download_and_ram_boot": [0x00121dc4, 0x00121dd0,
                                          0x001226b4, 0x001226fc,
                                          0x00122780, 0x00122928,
                                          0x0014d658, 0x0014d680,
                                          0x0014d684],
        "image_formats_and_handoff": [0x00101744, 0x001017fc, 0x0010e000,
                                      0x0010e02a, 0x0010e390, 0x0010f388,
                                      0x0010f728, 0x00127a1c],
        "boot_order_environment": [0x00100fe4, 0x001011e0, 0x001017e4,
                                   0x001095f8, 0x00110484,
                                   0x0010a058, 0x00118400],
        "recovery_buttons_and_reset": [0x00100e4a, 0x00100ee2,
                                       0x0010107e, 0x001010a0,
                                       0x001010aa, 0x001010f0,
                                       0x00101154, 0x00101296],
    }
}
REGISTERS = {
    "pmu_reboot": 0xff730094,
    "pmu_bootcount_new_hypothesis": 0xff7300a0,
    "pmu_base": 0xff730000,
    "usb_otg": 0xff580000,
    "uart2": 0xff690000,
    "efuse_secure": 0xffb10000,
    "efuse_ordinary": 0xffb40000,
    "rockchip_bootrom_magic": 0xef08a53c,
    "bootcount_magic_high_half": 0xb0010000,
    "legacy_uimage_magic": 0x27051956,
}
STRING_MARKERS = [
    ("unlock", b"inmusic-unlock-magic"), ("oem", b"oem"),
    ("permission", b"permission"), ("fastboot", b"fastboot"),
    ("download", b"download"), ("bootm", b"bootm"), ("bootz", b"bootz"),
    ("getvar", b"getvar"), ("mmc", b"mmc"), ("extlinux", b"extlinux"),
    ("sysboot", b"sysboot"), ("boot_targets", b"boot_targets"),
    ("bootcount", b"bootcount"), ("reset_bootloader", b"reset_bootloader"),
    ("saveenv", b"saveenv"), ("serial", b"serial"),
    ("recovery", b"recovery"), ("rockusb", b"rockusb"),
]
QUESTIONS = {
    "fastboot": [
        "Is the OEM unlock marker present in this exact P3 image?",
        "Which handler dispatches the OEM command, and what comparisons precede its success path?",
        "What writes the authorization global and what initializes its RAM value?",
        "Does authorization affect download, flash, erase and RAM boot equally?",
        "Does the OEM command modify saved eMMC environment or only RAM state?",
        "Does the RAM-boot path call bootm with a verified image format and FDT?",
        "What happens when bootm returns or image checksum validation fails?",
        "What RAM ranges overlap the download buffer, relocation, stack, FDT, framebuffer?",
        "Are read-only getvar commands usable before authorization?",
    ],
    "boot_selection": [
        "Can U-Boot select SD without an environment write?",
        "Is the serial/gpio-keys autoboot interruption reachable at bootdelay=0?",
        "Does boot_targets mmc0 mmc1 map to eMMC then SD on this exact board?",
        "What boot.scr/extlinux/bootz paths are actually executable?",
        "How are board revision and fdtfile chosen on MPC One ACVA?",
    ],
    "boot_counter": [
        "Is PMU 0xff7300a0 accessed by this older U-Boot?",
        "Is a b001 bootcount magic present, and is any threshold compared?",
        "Which kernel/userspace component should acknowledge successful boot?",
        "Does repeated warm reboot enter Fastboot independently of mode register 0xff730094?",
    ],
    "recovery": [
        "Which ADC/GPIO input triggers vendor Fastboot?",
        "Can a BootROM download request be consumed without altering eMMC?",
        "Which stage can survive failure of P6, P3 or P1?",
        "Is any firmware-signature check enforced on normal raw bootz startup?",
    ],
}
args = list(getScriptArgs())
if len(args) != 2 or str(args[0]) not in IMAGES:
    raise RuntimeError("Usage: p1|p3 /absolute/private/report.json")
role = str(args[0])
destination = Path(str(args[1]))
if not destination.is_absolute():
    raise RuntimeError("Report destination must be absolute")
base, size, want_hash = IMAGES[role]
program = currentProgram
if program is None or str(program.getLanguageID()) != "ARM:LE:32:v7":
    raise RuntimeError("Unexpected processor/language")
memory = program.getMemory()
space = program.getAddressFactory().getDefaultAddressSpace()
listing = program.getListing()
manager = program.getFunctionManager()
refs = program.getReferenceManager()
def A(value): return space.getAddress(int(value))
def H(value): return "0x%08x" % int(value)
block = memory.getBlock(A(base))
if block is None or block.getStart().getOffset() != base or block.getSize() != size:
    raise RuntimeError("Unexpected Ghidra memory image layout")
raw = bytearray()
chunk = JArray(JByte)(32768)
for offset in range(0, size, len(chunk)):
    count = min(len(chunk), size - offset)
    if int(memory.getBytes(A(base + offset), chunk, 0, count)) != count:
        raise RuntimeError("Unable to read entire verified firmware image")
    raw.extend(int(v) & 255 for v in chunk[:count])
if hashlib.sha256(raw).hexdigest() != want_hash:
    raise RuntimeError("Ghidra image does not match pinned owner SHA256")
def incoming(address, cap=64):
    found = []
    try:
        for r in refs.getReferencesTo(A(address)):
            source = int(r.getFromAddress().getOffset())
            fn = manager.getFunctionContaining(A(source))
            found.append({"from": H(source), "kind": str(r.getReferenceType()),
                          "function": H(fn.getEntryPoint().getOffset()) if fn else None})
            if len(found) >= cap:
                break
    except Exception as error:
        found.append({"error": str(error)[:200]})
    return found
def window(address, ahead=80, limit=42):
    result = []
    if not base <= address < base + size:
        return result
    instruction = listing.getInstructionContaining(A(max(base, address - 12)))
    if instruction is None:
        instruction = listing.getInstructionAfter(A(max(base, address - 12)))
    while instruction and len(result) < limit:
        pc = int(instruction.getAddress().getOffset())
        if pc >= base + size or pc > address + ahead:
            break
        result.append({"pc": H(pc), "asm": str(instruction)[:250],
                       "flows": [str(a) for a in instruction.getFlows()][:6]})
        instruction = instruction.getNext()
    return result
def redact(text):
    text = re.sub(r'(?i)inmusic-unlock-magic-[0-9a-f]+',
                  'inmusic-unlock-magic-[REDACTED]', text)
    text = re.sub(r'(?i)(password|secret|private.key|serialno|ethaddr)[^\n]{0,90}',
                  '[SENSITIVE_VALUE_REDACTED]', text)
    return text
out = {
    "schema": "mpfree-fastboot-directboot-ghidra-v12",
    "date_utc": datetime.now(timezone.utc).isoformat(),
    "role": role, "image_sha256": want_hash,
    "verified_read_only": True,
    "questions": QUESTIONS,
    "registers": {}, "markers": {}, "anchor_groups": {},
    "functions": {}, "limitations": [
        "The proprietary image and pseudocode must remain private.",
        "A literal/string/reference match cannot prove runtime reachability.",
        "Ghidra's function boundaries and ARM/Thumb mode can be wrong.",
        "A static BSS reference cannot establish its initial runtime value.",
        "No original vendor code was executed; no physical hardware was contacted.",
    ]
}
selected = {}
for name, address in REGISTERS.items():
    sequence = int(address).to_bytes(4, "little")
    places = [H(base + m.start()) for m in re.finditer(re.escape(sequence), bytes(raw))]
    x = incoming(address, 70)
    out["registers"][name] = {"address": H(address), "raw_literal_sites": places[:80],
                                "total_raw_literal_sites": len(places), "xrefs": x}
    for ref in x:
        addr = ref.get("function")
        if addr:
            fn = manager.getFunctionAt(A(int(addr, 16)))
            if fn: selected[int(addr, 16)] = fn
for name, marker in STRING_MARKERS:
    places = [base + m.start() for m in re.finditer(re.escape(marker), bytes(raw), re.I)]
    records = []
    for pos in places[:40]:
        rel = incoming(pos, 70)
        records.append({"address": H(pos), "references": rel[:50]})
        for ref in rel:
            addr = ref.get("function")
            if addr:
                fn = manager.getFunctionAt(A(int(addr, 16)))
                if fn: selected[int(addr, 16)] = fn
    out["markers"][name] = {"present": bool(places), "occurrences": len(places),
                            "sites": records}
for group, addresses in TARGETS[role].items():
    out["anchor_groups"][group] = {}
    for address in addresses:
        f = manager.getFunctionContaining(A(address))
        out["anchor_groups"][group][H(address)] = {
            "function": H(f.getEntryPoint().getOffset()) if f else None,
            "xrefs": incoming(address, 80),
            "disassembly": window(address),
        }
        if f: selected[int(f.getEntryPoint().getOffset())] = f
# Extend one caller hop for Fastboot entrypoints, but preserve anchor-first ordering.
for fn in list(selected.values()):
    for ref in refs.getReferencesTo(fn.getEntryPoint()):
        if not ref.getReferenceType().isCall():
            continue
        caller = manager.getFunctionContaining(ref.getFromAddress())
        if caller:
            selected.setdefault(int(caller.getEntryPoint().getOffset()), caller)
dec = DecompInterface()
dec.toggleCCode(True)
dec.setSimplificationStyle("decompile")
if not dec.openProgram(program):
    raise RuntimeError("Decompiler could not open program")
try:
    for address, fn in list(selected.items())[:115 if role == "p3" else 75]:
        record = {"address": H(address), "name": str(fn.getName()),
                  "incoming": incoming(address, 80), "calls": []}
        it = listing.getInstructions(fn.getBody(), True)
        while it.hasNext() and len(record["calls"]) < 160:
            ins = it.next()
            if ins.getFlowType().isCall() or ins.getFlowType().isJump():
                record["calls"].append({
                    "at": str(ins.getAddress()), "kind": str(ins.getFlowType()),
                    "targets": [str(t) for t in ins.getFlows()][:8]})
        try:
            result = dec.decompileFunction(fn, 35, monitor)
            if result and result.decompileCompleted() and result.getDecompiledFunction():
                source = redact(str(result.getDecompiledFunction().getC()))
                record["pseudocode"] = source[:16000]
                record["truncated"] = len(source) > 16000
            else:
                record["decompile_error"] = str(result.getErrorMessage()) if result else "No result"
        except Exception as exc:
            record["decompile_error"] = str(exc)[:350]
        out["functions"][H(address)] = record
finally:
    dec.dispose()
out["statistics"] = {
    "marker_categories_hit": sum(int(x["present"]) for x in out["markers"].values()),
    "decompiled_functions": sum(int("pseudocode" in x) for x in out["functions"].values()),
    "selected_functions": len(selected),
    "reported_functions": len(out["functions"]),
}
destination.parent.mkdir(parents=True, exist_ok=True)
old_umask = os.umask(0o077)
try:
    destination.write_text(json.dumps(out, indent=2) + "\n", encoding="utf-8")
finally:
    os.umask(old_umask)
print("V12 read-only", role, "report", destination,
      "functions", len(out["functions"]), "SHA256", want_hash)
