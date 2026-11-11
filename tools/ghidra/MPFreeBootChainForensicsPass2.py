# @runtime PyGhidra
# @category MPFree
# Offline P1/P3 follow-up: wide semantic analysis of every unresolved domain.
# Must be used with -readOnly -noanalysis on existing verified Ghidra projects.
import hashlib
import json
import os
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

from ghidra.app.decompiler import DecompInterface
from jpype import JArray, JByte

INPUTS = {
    "p1": (0xff704000, 32768,
           "6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"),
    "p3": (0x00100000, 363649,
           "b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"),
}
# Addresses come from owner-generated, verified *static* reports. Some point
# inside functions rather than at their starts, intentionally.
ANCHORS = {
 "p1": {
    "rom_start_and_stack": [0xff704020,0xff704064,0xff7040b4,0xff704068,
                            0xff704098,0xff705208,0xff705260,0xff705274],
    "boot_media_candidates": [0xff704120,0xff705342,0xff705350,0xff705398,
                               0xff7053c2,0xff7053d6,0xff7053fc,0xff708f98],
    "uboot_image_parser_and_handoff": [0xff7052c0,0xff70529c,0xff7052b8,
                                      0xff705308,0xff705310,0xff7054a4,
                                      0xff705598,0xff7055a4],
    "recovery_and_reboot": [0xff705316,0xff708b74,0xff708cdc,
                             0xff7056a8,0xff7041a8,0xff704160],
    "memory_ddr_and_peripherals": [0xff7055b0,0xff7055bc,0xff705444,
                                  0xff704164,0xff704204,0xff7041e8,
                                  0xff707f48,0xff7082c0],
 },
 "p3": {
    "board_reboot_register_and_modes": [0x00100e00,0x00100e4a,0x00100edc,
                                       0x00100ee2,0x00100fe4,0x0010107e,
                                       0x001010a0,0x001010dc,0x00101154],
    "recovery_adc_and_gpio": [0x001010aa,0x001010f0,0x00101100,
                               0x001011e0,0x00101296,0x001013b4,
                               0x00105620,0x00106000],
    "uboot_environment_and_bootcmd": [0x001017e4,0x001095f8,0x00110484,
                                     0x00108560,0x00108804,0x0010a058,
                                     0x00118400,0x00101300],
    "direct_boot_and_image_formats": [0x00101744,0x001017fc,0x00106376,
                                     0x0010a6a4,0x0010e390,0x0010e000,
                                     0x0010e02a,0x0010f388,0x0010f728,
                                     0x00127a1c],
    "fastboot_permission_state_and_ram": [0x001226b4,0x00122780,
                                        0x00122834,0x00122854,0x0012289e,
                                        0x00122948,0x00122964,0x00122980,
                                        0x00122a10,0x00121dc4,0x00121dd0,
                                        0x00122bf8,0x00122c5c,0x00122cf8],
    "fastboot_ram_memory_addresses": [0x0014d690,0x0014d680,0x0014d684,
                                     0x0014d658,0x00143e34,0x00122928,
                                     0x00122ac4,0x00122ad4,0x001226fc],
    "reset_and_survival_of_factory_recovery": [0x0011622a,0x00116216,
                                               0x00107050,0x0010733e,
                                               0x0010b74c,0x0010b824],
    "usb_otg_mmc_and_soc_security": [0x00120664,0x00120688,
                                    0x0011c3ac,0x0011cd54,
                                    0x001141e2,0x0010c134],
 },
}
MMIO = {
 "pmu_reboot_mode":0xff730094,
 "cru_softreset_or_boot":0xff760000,
 "usb_otg":0xff580000,
 "rk3288_efuse":0xffb10000,
 "rk3288_grf_candidate":0xff770320,
 "rk3288_uart2":0xff690000,
}
# These queries are PRIVATE, but explicitly avoid exporting authentication
# material, device identities and long proprietary string literals.
def redact(c):
    import re
    c = re.sub(r'"[^"\n]{25,}"', '"[LONG_STRING_REDACTED]"', c)
    c = re.sub(r'"[^"\n]*(?:unlock|password|secret|token|private.key|serialno|ethaddr)[^"\n]*"',
               '"[SENSITIVE_LITERAL_REDACTED]"', c, flags=re.I)
    return c
args = list(getScriptArgs())
if len(args)!=2 or str(args[0]) not in INPUTS:
    raise RuntimeError("Expected p1|p3 and absolute report path")
kind = str(args[0]); destination = Path(str(args[1]))
if not destination.is_absolute(): raise RuntimeError("Use absolute output path")
base, length, expected_hash = INPUTS[kind]
p = currentProgram
if p is None or str(p.getLanguageID()) != "ARM:LE:32:v7":
    raise RuntimeError("Wrong processor/program")
mem=p.getMemory(); space=p.getAddressFactory().getDefaultAddressSpace()
def A(n): return space.getAddress(int(n))
def H(n): return "0x%08x" % n
block=mem.getBlock(A(base))
if block is None or block.getStart().getOffset()!=base or block.getSize()!=length:
    raise RuntimeError("Wrong image base/size")
buf=JArray(JByte)(32768); raw=bytearray()
for pos in range(0,length,32768):
    n=min(length-pos,32768)
    if int(mem.getBytes(A(base+pos),buf,0,n))!=n:
        raise RuntimeError("Truncated memory block")
    raw.extend(int(x)&255 for x in buf[:n])
if hashlib.sha256(raw).hexdigest()!=expected_hash: raise RuntimeError("Unrecognized firmware hash")
fm=p.getFunctionManager(); listing=p.getListing(); refs=p.getReferenceManager()
def xrefs(n, cap=100):
    ans=[]
    try:
        for x in refs.getReferencesTo(A(n)):
            src=int(x.getFromAddress().getOffset())
            fun=fm.getFunctionContaining(A(src))
            ans.append({"from":H(src),"reference_type":str(x.getReferenceType()),
                        "function":H(fun.getEntryPoint().getOffset()) if fun else None})
            if len(ans)>=cap:break
    except Exception as e:
        return [{"error":str(e)}]
    return ans
def word(n):
    offset=n-base
    if 0<=offset<=length-4:
        return int.from_bytes(raw[offset:offset+4],"little")
    return None
def disasm(n, span=80, limit=90):
    ans=[]
    cursor=listing.getInstructionContaining(A(max(n-16,base)))
    if cursor is None: cursor=listing.getInstructionAfter(A(max(n-16,base)))
    while cursor is not None and len(ans)<limit:
        pc=cursor.getAddress().getOffset()
        if pc>n+span or pc>=base+length:break
        ans.append({"address":H(pc),"assembly":str(cursor),
                    "flows":[str(x) for x in cursor.getFlows()]})
        cursor=cursor.getNext()
    return ans
dec=DecompInterface()
dec.toggleCCode(True);dec.setSimplificationStyle("decompile")
if not dec.openProgram(p): raise RuntimeError("No decompiler")
out={
    "schema":"mpfree-bootchain-forensics-pass2-v1","kind":kind,
    "date_utc":datetime.now(timezone.utc).isoformat(),
    "status":"existing owner Ghidra project READ-ONLY; no hardware interaction",
    "payload_sha256":expected_hash,"image_base":H(base),"image_length":length,
    "anchors":{},"functions":{},"register_xrefs":{},"limitations":[
        "Function coverage is bounded to explicit anchors and referenced functions; no proof of unenumerated behavior.",
        "Pseudocode can be misleading at ARM/Thumb switches and indirect calls.",
        "Strings and literals are not evidence of code execution or of working silicon MaskROM.",
        "Ghidra data references to MMIO do not demonstrate that hardware writes occur.",
        "BSS/global file words are not initialized runtime values.",
        "Reports are private and must not be committed to the public repo.",
    ]
}
try:
    selected={}
    for group, addresses in ANCHORS[kind].items():
        out["anchors"][group]={}
        for address in addresses:
            if not (base<=address<base+length):
                out["anchors"][group][H(address)]={"outside_payload":True,"incoming":xrefs(address,80)}
                continue
            fn=fm.getFunctionContaining(A(address))
            record={"address":H(address),
                    "u32_at_address":H(word(address)) if word(address) is not None else None,
                    "incoming":xrefs(address,80),
                    "instructions":disasm(address,88)}
            if fn is not None:
                entry=int(fn.getEntryPoint().getOffset())
                record["containing_function"]=H(entry)
                selected[entry]=fn
            else:
                record["containing_function"]=None
            out["anchors"][group][H(address)]=record
    for name,address in MMIO.items():
        out["register_xrefs"][name]={"address":H(address),"references":xrefs(address,150)}
        for x in out["register_xrefs"][name]["references"]:
            if x.get("function"):
                entry=int(x["function"],16)
                fn=fm.getFunctionContaining(A(entry))
                if fn is not None: selected[entry]=fn
    # Follow *all* related callers up to one hop (not just named entry points).
    for fn in list(selected.values()):
        for ref in refs.getReferencesTo(fn.getEntryPoint()):
            if not ref.getReferenceType().isCall(): continue
            caller=fm.getFunctionContaining(ref.getFromAddress())
            if caller:
                entry=int(caller.getEntryPoint().getOffset())
                selected[entry]=caller
    for address,fn in sorted(selected.items()):
        if len(out["functions"])>=160:break
        entry={"address":H(address), "name":str(fn.getName()),
               "incoming":xrefs(address,50),"calls":[]}
        ins=listing.getInstructions(fn.getBody(),True)
        while ins.hasNext() and len(entry["calls"])<180:
            inst=ins.next()
            if inst.getFlowType().isCall() or inst.getFlowType().isJump():
                entry["calls"].append({"at":str(inst.getAddress()),
                                       "type":str(inst.getFlowType()),
                                       "targets":[str(t) for t in inst.getFlows()]})
        try:
            result=dec.decompileFunction(fn,90,monitor)
            if result and result.decompileCompleted() and result.getDecompiledFunction():
                source=redact(str(result.getDecompiledFunction().getC()))
                entry["pseudocode"]=source[:16000]
                entry["truncated"]=len(source)>16000
            else:
                entry["decompile_error"]=str(result.getErrorMessage()) if result else "No decompiler result"
        except Exception as e:
            entry["decompile_error"]=str(e)
        out["functions"][H(address)]=entry
finally: dec.dispose()
destination.parent.mkdir(parents=True,exist_ok=True)
old_umask=os.umask(0o077)
try:
    destination.write_text(json.dumps(out,indent=2)+"\n",encoding="utf-8")
finally: os.umask(old_umask)
print("MPFree P2 forensic pass:",kind,destination,
      "functions=",len(out["functions"]),"anchors=",sum(map(len,out["anchors"].values())))
