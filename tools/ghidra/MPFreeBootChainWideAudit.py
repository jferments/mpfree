# @runtime PyGhidra
# @category MPFree
# MPFree wide static boot-chain audit, both p1 SRAM SPL and p3 U-Boot proper.
# Reads pre-existing analyzed Ghidra projects with -readOnly -noanalysis.
# NO hardware, USB, eMMC, eFuse, reboot, flashing or project writes.
import hashlib
import json
import os
import re
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

from ghidra.app.decompiler import DecompInterface
from jpype import JArray, JByte

CONFIGS = {
    "p1": {
        "base": 0xff704000, "size": 32768,
        "sha256": "6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f",
        "groups": {
            "startup_and_handoff": [0xff704020, 0xff704060, 0xff704068, 0xff7040b8,
                                    0xff704118, 0xff705208, 0xff705260, 0xff705350,
                                    0xff705310, 0xff705298, 0xff705342],
            "boot_media_and_image_validation": [0xff704120, 0xff704164, 0xff7041a8,
                                                0xff70446c, 0xff7054a4, 0xff7052c0,
                                                0xff705418, 0xff7069e0, 0xff7082c0],
            "sram_and_dram": [0xff7041e8, 0xff7055b0, 0xff7055bc,
                              0xff705444, 0xff704204],
            "rom_return_and_reboot_candidates": [0xff705316, 0xff708b74,
                                                0xff7056a8, 0xff707f48],
        },
        "globals": [0xff705274, 0xff705268, 0xff705598, 0xff7053fc],
    },
    "p3": {
        "base": 0x00100000, "size": 363649,
        "sha256": "b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a",
        "groups": {
            "entry_board_init_and_boot_selection": [0x100000, 0x101744, 0x1017f4,
                   0x106376, 0x10733e, 0x108804, 0x108560, 0x1095f8,
                   0x110484, 0x116d14, 0x100edc, 0x100ef0],
            "bootm_kernel_fdt_and_memory": [0x10a6a4, 0x10a5c0, 0x10b140,
                   0x10e390, 0x10e000, 0x10e02a, 0x127a1c,
                   0x10f8a4, 0x10e600, 0x10dff4, 0x10a450],
            "fastboot_ram_boot": [0x1226b0, 0x1226b4, 0x122734, 0x122780,
                   0x122834, 0x12289e, 0x122948, 0x122964,
                   0x122980, 0x122a10, 0x12247c, 0x122284, 0x1224a0],
            "fastboot_oem_update_and_reboot": [0x122854, 0x122bf8, 0x122c5c,
                   0x122cf8, 0x122d38, 0x11622a, 0x116216,
                   0x121dc4, 0x121dd0, 0x121ddc, 0x10701e],
            "storage_recovery_usb_and_environment": [0x1100c8, 0x110270,
                   0x10cc64, 0x107308, 0x107342, 0x107010,
                   0x107070, 0x1070a4, 0x10e9f4],
        },
        "globals": [0x14d690, 0x14d680, 0x14d684, 0x14d658,
                    0x143e34, 0x12f2b0],
    },
}

# Strings or immediate values are only leads. A match never demonstrates
# executable reachability, silicon behavior or an authorized operation.
KEYWORDS = (
    "maskrom", "brom", "bootrom", "rockusb", "rockchip", "fastboot",
    "bootcmd", "boot_targets", "distro_bootcmd", "bootdelay", "boot.scr",
    "extlinux", "bootz", "bootm", "booti", "tftp", "usb start",
    "usb reset", "mmc", "emmc", "sdcard", "bootpart", "uboot",
    "u-boot", "recovery", "dorecovery", "reset_bootloader",
    "reboot", "boot_mode", "boot-mode", "boot mode", "loader",
    "download", "bootargs", "fdtfile", "fdt ", "fit", "image",
    "verified", "secureboot", "secure boot", "rsa", "sha256",
    "efuse", "otp", "vqps", "serial", "gpio", "adc", "dwc2",
    "ff580000", "ffb10000", "rk32", "idblock", "rc4",
)
REDACT = re.compile(r"unlock|password|passphrase|secret|token|credential|certificate|private.key|"
                    r"auth.magic|serialno|serial#|ethaddr|macaddr|inmusic-unlock", re.I)
INTERESTING_WORDS = {
    "brom_download_magic": 0xef08a53c,
    "legacy_uimage_magic": 0x27051956,
    "legacy_uimage_le_word": 0x56190527,
    "rk32_le_word": 0x32334b52,
    "idblock_signature": 0x0ff0aa55,
    "rk3288_usb_otg_base": 0xff580000,
    "rk3288_efuse_base": 0xffb10000,
    "p3_start_sector": 0x100,
}
QUESTIONS = {
    "maskrom_entry": "Locate *actual* mode-register writes and reset path in p1/p3; the immutable silicon ROM is absent. A missing literal is NOT proof of no ROM re-entry.",
    "rom_usb_capability": "Confirm ff580000 USB-OTG initialization and board routing from DTB, not strings alone; only actual MaskROM enumeration proves the state.",
    "p1_to_p3_handoff": "Follow 0xff705260 indirect Thumb jump, 0xff7054a4 sector reader, 0xff7052c0 image fields, 0xff705310 final execution address.",
    "alternate_boot": "Trace U-Boot bootcmd -> distro_bootcmd -> boot_targets, extlinux/boot.scr parsing, mmc/USB probing and board key/ADC override.",
    "fastboot_ram_boot": "Resolve runtime permission initializer, download buffer bounds, boot/continue state transitions, bootm validation and execution.",
    "environment_safety": "Identify environment CRC, default-vs-saved values, CRC failure fallback and whether a RAM-only environment override is possible.",
    "secure_boot": "Identify signature/efuse operations vs CRC32 only; inspect board OTP policy and vendor updater separately, without programming fuses.",
    "memory_ownership": "Establish relocated U-Boot, stack, DRAM banks, kernel/FDT/initrd/download addresses and overlap checks before any RAM boot.",
    "recovery": "Separate U-Boot Fastboot, Rockusb loader and silicon MaskROM; determine p1/p3 independence from p6; UART and recovery key mapping.",
    "firmware_updates": "Inspect owner backups/update packages for any p1/p2/p3 writes, A/B assumptions, bootloader signatures, OTP access and rollback protection.",
}

args = list(getScriptArgs())
if len(args) != 2 or args[0] not in CONFIGS:
    raise RuntimeError("Expected -postScript MPFreeBootChainWideAudit.py p1|p3 /absolute/report.json")
kind = str(args[0])
output_path = Path(str(args[1])).expanduser()
if not output_path.is_absolute():
    raise RuntimeError("Output path must be absolute")
cfg = CONFIGS[kind]
program = currentProgram
if program is None or str(program.getLanguageID()) != "ARM:LE:32:v7":
    raise RuntimeError("Wrong Ghidra program or processor; expected ARM:LE:32:v7")
mem = program.getMemory()
space = program.getAddressFactory().getDefaultAddressSpace()
base = cfg["base"]
size = cfg["size"]
def A(value):
    return space.getAddress(int(value))

block = mem.getBlock(A(base))
if block is None or block.getStart().getOffset() != base or block.getSize() != size:
    raise RuntimeError("Wrong backing memory block at %08x, length %d" % (base, size))
buf = JArray(JByte)(32768)
payload = bytearray()
for position in range(0, size, 32768):
    needed = min(32768, size - position)
    got = int(mem.getBytes(A(base + position), buf, 0, needed))
    if got != needed:
        raise RuntimeError("Truncated Ghidra block at %x" % position)
    payload.extend(int(x) & 255 for x in buf[:needed])
digest = hashlib.sha256(payload).hexdigest()
if digest != cfg["sha256"]:
    raise RuntimeError("Wrong p1/p3 payload SHA256 %s (refusing analysis)" % digest)

listing = program.getListing()
fm = program.getFunctionManager()
refs = program.getReferenceManager()
def hexaddr(n):
    return "0x%08x" % int(n)
def read_u32(va):
    off = int(va) - base
    if 0 <= off <= len(payload) - 4:
        return int.from_bytes(payload[off:off+4], "little")
    return None
def allrefs(at, limit=80):
    out = []
    try:
        for ref in refs.getReferencesTo(A(at)):
            out.append({"from": str(ref.getFromAddress()), "type": str(ref.getReferenceType())})
            if len(out) >= limit: break
    except Exception as e:
        return [{"error": str(e)}]
    return out
def instructions_near(at, before=16, after=50):
    # Include only *existing* disassembly. Do not use disassemble() in readonly mode.
    start = max(base, int(at)-before)
    end = min(base+size, int(at)+after)
    out = []
    current = listing.getInstructionContaining(A(start))
    if current is None:
        current = listing.getInstructionAfter(A(start))
    while current and current.getAddress().getOffset() < end and len(out) < 40:
        pc = current.getAddress().getOffset()
        if pc >= start:
            out.append({"at": hexaddr(pc), "asm": str(current),
                        "flows": [str(x) for x in current.getFlows()]})
        current = current.getNext()
    return out

result = {
    "schema": "mpfree-wide-bootchain-audit-v1",
    "kind": kind, "program": str(program.getName()),
    "image_base": str(program.getImageBase()),
    "payload_base": hexaddr(base), "payload_bytes": size, "sha256": digest,
    "date_utc": datetime.now(timezone.utc).isoformat(),
    "status": "OFFLINE STATIC OBSERVATION ONLY",
    "safety": [
        "No device communication, USB commands, MaskROM request or boot-media writes.",
        "No Ghidra project modifications; launcher uses -readOnly -noanalysis.",
        "OEM bytes, vendor strings and all decompilation are PRIVATE; never commit this report.",
        "On-disk values of RAM globals do NOT establish runtime values.",
        "Disassembler ARM/Thumb modes, indirect calls and xrefs may be incomplete.",
        "Absent magic literals, strings or xrefs do NOT establish absent functionality.",
        "MaskROM itself is immutable silicon code and is not present in p1/p3.",
    ],
    "open_question_matrix": QUESTIONS,
    "focus_functions": {}, "globals": {}, "literal_hits": {},
    "strings": [], "pointer_candidates": [], "program_statistics": {},
    "instruction_windows": {},
}
# Exact payload byte-pattern scan in both endian forms.
for label, word in INTERESTING_WORDS.items():
    hits = []
    for endian in ("little", "big"):
        pattern = word.to_bytes(4, endian)
        pos = payload.find(pattern)
        while pos >= 0 and len(hits) < 100:
            hits.append({"address": hexaddr(base+pos), "endian": endian,
                         "aligned4": bool(pos % 4 == 0)})
            pos = payload.find(pattern, pos+1)
    result["literal_hits"][label] = hits

# ASCII scan: bound volume; strings are *candidate evidence* only.
seen = set()
for match in re.finditer(rb"[\x20-\x7e]{4,}", bytes(payload)):
    raw = match.group(0).decode("ascii", "replace")
    low = raw.lower()
    categories = [term for term in KEYWORDS if term in low]
    if not categories:
        continue
    va = base + match.start()
    if va in seen: continue
    seen.add(va)
    if REDACT.search(raw):
        safe = "[sensitive literal redacted; %d printable bytes]" % len(raw)
    else:
        safe = raw[:150] + (" [truncated]" if len(raw) > 150 else "")
    result["strings"].append({
        "at": hexaddr(va), "text": safe, "keywords": categories[:10],
        "xrefs": allrefs(va, 25),
    })
    if len(result["strings"]) >= 3000:
        result["string_list_truncated"] = True
        break

# Enumerate 32-bit data-looking Thumb pointers, not a reachability claim.
if kind == "p1":
    for off in range(0, len(payload)-3, 4):
        word = int.from_bytes(payload[off:off+4], "little")
        target = word & ~1
        if (word & 1) and base <= target < base+size:
            fn = fm.getFunctionContaining(A(target))
            result["pointer_candidates"].append({
                "source": hexaddr(base+off), "word": hexaddr(word),
                "thumb_target": hexaddr(target),
                "function": str(fn.getName()) if fn else None,
            })
            if len(result["pointer_candidates"]) >= 350:
                result["pointer_candidates_truncated"] = True
                break

dec = DecompInterface()
dec.toggleCCode(True)
dec.setSimplificationStyle("decompile")
if not dec.openProgram(program):
    raise RuntimeError("Decompiler cannot open existing Ghidra project")

try:
    for group, addresses in cfg["groups"].items():
        result["focus_functions"][group] = {}
        for va in addresses:
            entry = {"address": hexaddr(va), "references": allrefs(va, 60)}
            fun = fm.getFunctionAt(A(va))
            if fun is None:
                containing = fm.getFunctionContaining(A(va))
                entry["recognized_function"] = False
                entry["containing_function"] = (hexaddr(containing.getEntryPoint().getOffset())
                                                if containing else None)
            else:
                entry["recognized_function"] = True
                entry["name"] = str(fun.getName())
                entry["function_bytes"] = int(fun.getBody().getNumAddresses())
                entry["calls"] = []
                it = listing.getInstructions(fun.getBody(), True)
                while it.hasNext() and len(entry["calls"]) < 180:
                    ins = it.next()
                    if ins.getFlowType().isCall():
                        entry["calls"].append({
                            "at": str(ins.getAddress()),
                            "targets": [str(x) for x in ins.getFlows()],
                            "type": str(ins.getFlowType()),
                        })
                try:
                    dr = dec.decompileFunction(fun, 75, monitor)
                    if dr and dr.decompileCompleted() and dr.getDecompiledFunction():
                        source = str(dr.getDecompiledFunction().getC())
                        source = re.sub(r'"[^"\n]{28,}"',
                                        '"[LONG_LITERAL_REDACTED]"', source)
                        source = re.sub(r'"[^"\n]*(?:unlock|password|secret|token|serialno)[^"\n]*"',
                                        '"[SENSITIVE_LITERAL_REDACTED]"', source, flags=re.I)
                        entry["pseudocode"] = source[:18000]
                        entry["pseudocode_truncated"] = len(source) > 18000
                    else:
                        entry["decompile_error"] = str(dr.getErrorMessage()) if dr else "No result"
                except Exception as error:
                    entry["decompile_error"] = str(error)
            entry["local_disassembly"] = instructions_near(va)
            result["focus_functions"][group][hexaddr(va)] = entry
    # Ghidra sometimes associates globals with an uninitialized BSS overlay.
    # Only inspect a raw word when it resides in verified image payload.
    for va in cfg["globals"]:
        result["globals"][hexaddr(va)] = {
            "raw_file_value": (hexaddr(read_u32(va)) if read_u32(va) is not None else None),
            "raw_value_is_runtime_state": False,
            "references": allrefs(va, 200),
            "instructions_near": instructions_near(va, 8, 20),
        }
    count = 0
    function_names = []
    functions = fm.getFunctions(True)
    while functions.hasNext():
        f = functions.next()
        count += 1
        name = str(f.getName())
        if any(term in name.lower() for term in ("boot", "rom", "usb", "reset", "fastboot", "env")):
            function_names.append({"at": str(f.getEntryPoint()), "name": name})
    result["program_statistics"]["functions_known_to_ghidra"] = count
    result["program_statistics"]["interesting_named_functions"] = function_names[:200]

    # A second, automatically discovered wave: follow string xrefs into
    # functions not known by hard-coded addresses. This is useful for
    # board-specific bootcmd, ROM-return, storage and recovery branches.
    already = set()
    for by_address in result["focus_functions"].values():
        for row in by_address.values():
            if row["recognized_function"]:
                already.add(int(row["address"], 16))
    candidate_terms = ("boot_targets", "distro_bootcmd", "boot.scr", "extlinux",
                       "recovery", "reset_bootloader", "maskrom", "brom",
                       "rockusb", "efuse", "secure", "boot_mode", "bootloader",
                       "mmc", "usb start", "fit", "fdt")
    candidates = {}
    for row in result["strings"]:
        if not any(term in row["keywords"] or term in row["text"].lower()
                   for term in candidate_terms):
            continue
        for ref in row["xrefs"][:25]:
            try:
                at = int(ref["from"], 16)
                fun = fm.getFunctionContaining(A(at))
                if fun is None: continue
                entry = fun.getEntryPoint().getOffset()
                if entry not in already:
                    candidates[entry] = fun
            except Exception:
                continue
    result["auto_discovered_functions"] = {}
    for va in sorted(candidates)[:60]:
        fun = candidates[va]
        row = {"address": hexaddr(va), "name": str(fun.getName()),
               "incoming": allrefs(va, 30)}
        try:
            dr = dec.decompileFunction(fun, 65, monitor)
            if dr and dr.decompileCompleted() and dr.getDecompiledFunction():
                source = str(dr.getDecompiledFunction().getC())
                source = re.sub(r'"[^"\n]{28,}"', '"[LONG_LITERAL_REDACTED]"', source)
                source = re.sub(r'"[^"\n]*(?:unlock|password|secret|token|serialno)[^"\n]*"',
                                '"[SENSITIVE_LITERAL_REDACTED]"', source, flags=re.I)
                row["pseudocode"] = source[:12000]
                row["truncated"] = len(source) > 12000
            else:
                row["error"] = str(dr.getErrorMessage()) if dr else "no result"
        except Exception as error:
            row["error"] = str(error)
        result["auto_discovered_functions"][hexaddr(va)] = row
    result["auto_discovered_candidates_total"] = len(candidates)

    # Program-wide Ghidra xrefs to SoC-address space. These are leads,
    # not proof that a physical MMIO transaction occurs.
    mmio_sites = []
    mmio_counts = Counter()
    instruction_count = 0
    all_instructions = listing.getInstructions(True)
    while all_instructions.hasNext() and instruction_count < 90000:
        ins = all_instructions.next()
        instruction_count += 1
        try:
            for ref in refs.getReferencesFrom(ins.getAddress()):
                target = int(ref.getToAddress().getOffset())
                # Exclude sign-extended negative/sentinel values near 0xffffffff.
                # Include plausible SoC MMIO and the ARM high-vector anchor.
                if not (0xff000000 <= target <= 0xffefffff or target == 0xffff0000) or base <= target < base+size:
                    continue
                mmio_counts[hexaddr(target)] += 1
                if len(mmio_sites) < 350:
                    mmio_sites.append({"instruction": str(ins.getAddress()),
                                       "target": hexaddr(target),
                                       "asm": str(ins),
                                       "type": str(ref.getReferenceType())})
        except Exception:
            continue
    result["mmio_reference_candidates"] = mmio_sites
    result["mmio_reference_summary"] = mmio_counts.most_common(100)
    result["program_statistics"]["instructions_scanned_for_mmio_refs"] = instruction_count
finally:
    dec.dispose()

# Useful static anchors: no claim of hardware behavior.
for label, addresses in cfg["groups"].items():
    for va in addresses[:3]:
        result["instruction_windows"][label+"_"+hexaddr(va)] = instructions_near(va, 16, 96)

output_path.parent.mkdir(parents=True, exist_ok=True)
# Defensive: owner-only permissions; contents are proprietary and may be sensitive.
old_umask = os.umask(0o077)
try:
    with output_path.open("w", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2)
        handle.write("\n")
finally:
    os.umask(old_umask)
print("MPFree WIDE BOOTCHAIN AUDIT: %s" % output_path)
print("Kind=%s; SHA256=%s; functions=%d; strings=%d" % (
    kind, digest, result["program_statistics"]["functions_known_to_ghidra"],
    len(result["strings"])))
