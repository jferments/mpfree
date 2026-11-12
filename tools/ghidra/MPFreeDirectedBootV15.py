# @runtime PyGhidra
# @category MPFree
"""MPFree V15 directed read-only ARM disassembly/dataflow.

Verifies exact saved-owner Ghidra image SHA; traverses selected function
instructions, references, literal pointers, and P-code dataflow.
P3 can export ONE 4096-byte verified code page to a caller-designated
private temporary directory for ISOLATED optional ARM user-mode tests.
No owner firmware executable is invoked by this script.
"""
import hashlib
import json
import os
from pathlib import Path
from datetime import datetime, timezone
from ghidra.app.decompiler import DecompInterface
from jpype import JArray,JByte

INPUT={
 "p1":(0xff704000,32768,"6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"),
 "p3":(0x00100000,363649,"b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a")
}
SEEDS={
 "p3":{
  "oem_auth_command":[0x00122834,0x00122854,0x0012dc78],
  "oem_command_interpreter":[0x00122854,0x00110484,0x0012e950,0x0012dc78],
  "usb_download":[0x00122980,0x00122a10,0x0012247c,0x00121dd0],
  "ram_boot":[0x00122964,0x001226b4,0x00101744,0x0010d14c,0x0011622a],
  "image_validation":[0x0010e390,0x0010a6a4,0x0010e000,0x0010e02a,0x0010f8a4,0x00127a1c],
  "mmc_boot":[0x00100fe4,0x001011e0,0x00110484,0x001095f8,0x0010e390],
  "recovery_policy":[0x00100fe4,0x0010b74c,0x0011622a,0x00108560],
  "getvar":[0x00122d38,0x00122780],
 },
 "p1":{
  "rom_entry":[0xff704004,0xff704060,0xff704068,0xff7040b4],
  "media_order":[0xff705310,0xff705350,0xff705398,0xff7053c2,0xff7054a4],
  "handoff":[0xff705208,0xff7052c0,0xff705308,0xff705598,0xff7055a4],
  "recovery":[0xff705316,0xff708b74,0xff708cdc,0xff7056a8],
 }
}
POINTERS={
 "p3":{
  "permission_reader_literal":0x0012284c,
  "unlock_writer_literal":0x00122928,
  "unlock_text_literal":0x00122924,
  "download_size_literal":0x001229f4,
  "download_count_literal":0x001229f8,
  "receive_count_literal":0x00122ac4,
  "receive_size_literal":0x00122ad4,
  "boot_image_addr_literal":0x001226fc,
  "bootm_format_literal":0x00122700,
  "dispatch_table_literal":0x00122818,
 },
 "p1":{},
}
GLOBALS={
 "p3":{
  "permission_global":0x0014d690,
  "reboot_mode_register":0xff730094,
  "bootcounter_register_candidate":0xff7300a0,
  "kernel_addr":0x02000000,
  "fdt_addr":0x01f00000,
  "ramdisk_addr":0x04000000,
  "download_addr":0x00800800,
 },
 "p1":{
  "reboot_mode_register":0xff730094,
  "bootcounter_register_candidate":0xff7300a0,
  "rom_download_magic":0xef08a53c,
 }
}
argv=list(getScriptArgs())
if len(argv)!=3:raise RuntimeError("role p1|p3 absolute-report-json absolute-temp-page-or-none")
role=str(argv[0]);report_file=Path(str(argv[1]));page_arg=str(argv[2])
if role not in INPUT or not report_file.is_absolute():raise RuntimeError("Invalid role/report")
if page_arg!="none" and (role!="p3" or not Path(page_arg).is_absolute()):
    raise RuntimeError("Page export only for P3 and absolute private temp path")
base,length,expected=INPUT[role]
program=currentProgram
if program is None or str(program.getLanguageID())!="ARM:LE:32:v7":
    raise RuntimeError("Unexpected Ghidra architecture")
space=program.getAddressFactory().getDefaultAddressSpace()
def A(x):return space.getAddress(int(x))
def H(x):return "0x%08x"%int(x)
mem=program.getMemory();block=mem.getBlock(A(base))
if block is None or block.getStart().getOffset()!=base or block.getSize()!=length:
    raise RuntimeError("Saved project memory-image size/base mismatch")
buf=JArray(JByte)(32768);data=bytearray()
for off in range(0,length,32768):
    n=min(32768,length-off)
    if int(mem.getBytes(A(base+off),buf,0,n))!=n:raise RuntimeError("Failed to read owner image")
    data.extend(int(x)&255 for x in buf[:n])
raw=bytes(data)
if hashlib.sha256(raw).hexdigest()!=expected:raise RuntimeError("Saved firmware SHA256 mismatch")
fm=program.getFunctionManager();refs=program.getReferenceManager();listing=program.getListing()
def read32(at):
    if base<=at<=base+length-4:return int.from_bytes(raw[at-base:at-base+4],"little")
    return None
def xrefs(at,cap=500):
    found=[]
    try:
        for ref in refs.getReferencesTo(A(at)):
            loc=int(ref.getFromAddress().getOffset())
            fn=fm.getFunctionContaining(A(loc))
            found.append({"at":H(loc),"caller":H(fn.getEntryPoint().getOffset()) if fn else None,
                          "type":str(ref.getReferenceType())})
            if len(found)>=cap:break
    except Exception as e:found.append({"error":str(e)[:200]})
    return found
def raw_literal_sites(word,limit=250):
    needle=int(word).to_bytes(4,"little");found=[];offset=0
    while len(found)<limit:
        offset=raw.find(needle,offset)
        if offset<0:break
        found.append(H(base+offset));offset+=1
    return found
def redact(s):
    import re
    return re.sub(r"(?i)inmusic-unlock-magic-[0-9a-f]+","inmusic-unlock-magic-[REDACTED]",s)
def instructions(fn,cap=420):
    result=[];it=listing.getInstructions(fn.getBody(),True)
    while it.hasNext() and len(result)<cap:
        ins=it.next()
        result.append({"pc":H(ins.getAddress().getOffset()),"asm":str(ins)[:180],
                       "targets":[H(t.getOffset()) for t in ins.getFlows()][:8]})
    return result
report={
 "schema":"mpfree-v15-directed-ghidra-v1","role":role,
 "datetime_utc":datetime.now(timezone.utc).isoformat(),
 "source_sha256":expected,"image_hash_verified":True,
 "static_only":True,"device_access":False,
 "pointer_cells":{},"watch_globals":{},"domains":{},
 "function_details":{},"unresolved_indirect":[],
 "cautions":[
  "Ghidra high P-code and decompilation are static approximations.",
  "A global pointer alias is not a physical runtime authorization proof.",
  "No vendor code executes in Ghidra.",
  "The optional code page is PRIVATE TEMP ONLY; it must never be archived.",
 ]
}
for name,loc in POINTERS[role].items():
    val=read32(loc)
    report["pointer_cells"][name]={
        "literal_address":H(loc),"value":H(val) if val is not None else None,
        "references":xrefs(loc,120),
        "target_references":xrefs(val,120) if val is not None else [],
    }
for name,loc in GLOBALS[role].items():
    report["watch_globals"][name]={
        "address":H(loc),"ghidra_refs":xrefs(loc,400),
        "raw_little_endian_literal_sites":raw_literal_sites(loc),
    }
if role=="p3":
    readval=read32(POINTERS[role]["permission_reader_literal"])
    writeval=read32(POINTERS[role]["unlock_writer_literal"])
    report["permission_alias"]={
        "read_pointer":H(readval),"write_pointer":H(writeval),
        "same_nonzero_address":bool(readval and readval==writeval),
    }
    # Ghidra may label Thumb functions with even entry offsets; table stores odd ptr.
    table=0x0012f2b0
    entries=[]
    for i in range(8):
        key=read32(table+8*i);fnptr=read32(table+8*i+4)
        token=None
        if key is not None and base<=key<base+length:
            b=raw[key-base:key-base+22].split(b"\x00",1)[0]
            try:token=b.decode("ascii")
            except Exception:token=None
        entries.append({"slot":i,"command":token,"handler":H(fnptr) if fnptr is not None else None})
    report["command_dispatch"]=entries
selected={}
for domain,addresses in SEEDS[role].items():
    resolved=[]
    for at in addresses:
        f=fm.getFunctionContaining(A(at))
        if f:
            entry=int(f.getEntryPoint().getOffset());selected[entry]=f
            resolved.append(H(entry))
    report["domains"][domain]={"seeds":[H(x) for x in addresses],
                                "resolved_functions":sorted(set(resolved))}
# Expand immediate callers and callees around the domain roots.
roots=set(selected)
for fn in list(selected.values()):
    for ref in refs.getReferencesTo(fn.getEntryPoint()):
        if not ref.getReferenceType().isCall():continue
        caller=fm.getFunctionContaining(ref.getFromAddress())
        if caller:
            at=int(caller.getEntryPoint().getOffset());selected.setdefault(at,caller)
    it=listing.getInstructions(fn.getBody(),True)
    while it.hasNext():
        ins=it.next()
        if ins.getFlowType().isCall():
            for target in ins.getFlows():
                callee=fm.getFunctionContaining(target)
                if callee:
                    at=int(callee.getEntryPoint().getOffset());selected.setdefault(at,callee)
# Deliberately cap worst-case output but report when the cap is reached.
dec=DecompInterface();dec.toggleCCode(True);dec.setSimplificationStyle("decompile")
if not dec.openProgram(program):raise RuntimeError("Cannot open verified program")
try:
    for at,fn in list(sorted(selected.items(),key=lambda x:(x[0] not in roots,x[0])))[:145]:
        item={"function":H(at),"name":str(fn.getName()),
              "instructions":instructions(fn),"incoming":xrefs(at,100)}
        it=listing.getInstructions(fn.getBody(),True);calls=[];unresolved=[]
        while it.hasNext():
            ins=it.next()
            if not ins.getFlowType().isCall():continue
            dst=[H(x.getOffset()) for x in ins.getFlows()]
            calls.append({"site":H(ins.getAddress().getOffset()),"targets":dst})
            if not dst:
                unresolved.append({"function":H(at),"site":H(ins.getAddress().getOffset()),
                                   "assembly":str(ins)})
        item["calls"]=calls
        report["unresolved_indirect"].extend(unresolved)
        try:
            result=dec.decompileFunction(fn,30,monitor)
            if result and result.decompileCompleted() and result.getDecompiledFunction():
                c=redact(str(result.getDecompiledFunction().getC()))
                item["pseudocode"]=c[:16000]
                item["pseudocode_truncated"]=len(c)>16000
                hf=result.getHighFunction()
                pcode=[]
                if hf is not None:
                    ops=hf.getPcodeOps()
                    while ops.hasNext() and len(pcode)<700:
                        op=ops.next()
                        opcode=op.getMnemonic()
                        if opcode not in ("CALL","CALLIND","STORE","LOAD","COPY","CBRANCH","BRANCHIND","INT_EQUAL","INT_NOTEQUAL"):
                            continue
                        args=[str(op.getInput(i))[:90] for i in range(op.getNumInputs())]
                        pcode.append({"pc":str(op.getSeqnum().getTarget()),
                                      "op":opcode,"inputs":args,
                                      "output":str(op.getOutput())[:90] if op.getOutput() else None})
                item["pcode_critical"]=pcode
            else:item["decompile_error"]=str(result.getErrorMessage())[:300] if result else "No result"
        except Exception as exc:item["decompile_error"]=str(exc)[:300]
        report["function_details"][H(at)]=item
finally:dec.dispose()
report["statistics"]={
    "root_functions":len(roots),"expanded_functions":len(selected),
    "reported_functions":len(report["function_details"]),
    "pcode_operations":sum(len(x.get("pcode_critical",[])) for x in report["function_details"].values()),
    "unresolved_indirect_sites":len(report["unresolved_indirect"])
}
if page_arg!="none":
    PAGE=0x00122000
    if PAGE<base or PAGE+4096>base+length:raise RuntimeError("Private code slice not within pinned owner image")
    path=Path(page_arg)
    if path.is_symlink():raise RuntimeError("Refusing symlink for private page")
    path.parent.mkdir(parents=True,exist_ok=True)
    old_umask=os.umask(0o077)
    try:
        with path.open("xb") as f:f.write(raw[PAGE-base:PAGE-base+4096])
        os.chmod(str(path),0o600)
    finally:os.umask(old_umask)
    report["isolated_code_page"]={
        "source_start":H(PAGE),"length":4096,
        "sha256":hashlib.sha256(raw[PAGE-base:PAGE-base+4096]).hexdigest(),
        "private_temp_only":True,"included_in_zip":False,
        "permitted_emulated_function":"mpf_fastboot_permission 0x00122834 only"
    }
report_file.parent.mkdir(parents=True,exist_ok=True)
old_umask=os.umask(0o077)
try:report_file.write_text(json.dumps(report,indent=2)+"\n",encoding="utf-8")
finally:os.umask(old_umask)
print("MPFree V15 directed",role,"selected",len(selected),
      "pcode",report["statistics"]["pcode_operations"],"source SHA verified")
